#!/usr/bin/env node
/**
 * Command-line assembly only.
 *
 * Every command maps options onto one operation of `service.ts`; document
 * semantics, saving and export stay native. Help text and errors come from
 * `@wae/commander`, so the CLI does not grow its own parser.
 */

import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

import { createCli } from '@wae/commander';

import * as service from './service.js';
import { runGui } from './gui.js';
import type { Document, DocumentCapabilities, EditOperation, ExportFormat, ExportReport, InspectReport } from './types.js';
import type { InspectCheck } from './types.js';

const here = path.dirname(fileURLToPath(import.meta.url));

const cli = createCli('open-word').locales(path.join(here, '..', 'locales')).intro('cli.intro');

// Exported for contract audits that import the built entry to inspect the
// command tree. Nothing runs until the file is executed as the program entry.
export { cli };

interface ParsedOptions {
    readonly _?: readonly string[];
    readonly [key: string]: unknown;
}

const text = (options: ParsedOptions, key: string): string | undefined =>
    typeof options[key] === 'string' ? (options[key] as string) : undefined;

const flag = (options: ParsedOptions, key: string): boolean => options[key] === true || options[key] === 'true';

const input = (options: ParsedOptions): string | undefined => options._?.[0];

function parseInteger(value: string | undefined): number | undefined {
    if (value === undefined) {
        return undefined;
    }
    const parsed = Number.parseInt(value, 10);
    return Number.isInteger(parsed) && parsed >= 0 ? parsed : undefined;
}

function parseRows(value: string): string[][] | undefined {
    let parsed: unknown;
    try {
        parsed = JSON.parse(value);
    } catch {
        return undefined;
    }
    if (!Array.isArray(parsed) || parsed.length === 0) {
        return undefined;
    }
    const rows: string[][] = [];
    for (const row of parsed) {
        if (!Array.isArray(row) || row.length === 0) {
            return undefined;
        }
        const cells: string[] = [];
        for (const cell of row) {
            if (typeof cell !== 'string') {
                return undefined;
            }
            cells.push(cell);
        }
        rows.push(cells);
    }
    return rows;
}

function describeCapabilities(matrix: DocumentCapabilities): string {
    const formats = Object.entries(matrix.formats)
        .filter(([, entry]) => entry.import === 'supported')
        .map(([name]) => name);
    const checks = Object.entries(matrix.checks)
        .filter(([, level]) => level === 'supported')
        .map(([name]) => name);
    return [
        `engine: ${matrix.engine.package} ${matrix.engine.version}`,
        `project: ${matrix.project}`,
        `document formats: ${formats.length > 0 ? formats.join(', ') : 'none supported'}`,
        `checks: ${checks.length > 0 ? checks.join(', ') : 'none supported'}`,
    ].join('\n');
}

function describeDocument(document: Document): string {
    return [
        `document ${document.id} revision=${document.revision} sections=${document.sections.length} paragraphs=${document.paragraphs.length} tables=${document.tables.length}`,
        `  title: ${document.title ?? '(none)'}`,
    ].join('\n');
}

function describeReport(report: InspectReport): string {
    const lines = [`inspect revision=${report.revision} checker=${report.checkerVersion} passed=${report.passed}`];
    for (const issue of report.issues) {
        lines.push(`  ${issue.severity} ${issue.code} ${issue.sectionId ?? '-'} ${issue.message}`);
    }
    return lines.join('\n');
}

function describeExport(report: ExportReport): string {
    return `export ${report.format} -> ${report.target} revision=${report.revision} editable=${report.editable} losses=${report.losses.length}`;
}

cli.command('capabilities', 'cli.cmd.capabilities').actionCtx(async (_options, _args, ctx) => {
    const matrix = service.capabilities();
    ctx.emitResult(matrix, describeCapabilities(matrix));
    return 0;
});

cli
    .command('create', 'cli.cmd.create')
    .option('--out <file>', 'cli.opt.out')
    .option('--title <title>', 'cli.opt.title')
    .option('--template <file>', 'cli.opt.template')
    .option('--overwrite', 'cli.opt.overwrite')
    .actionCtx(async (options, _args, ctx) => {
        const out = text(options, 'out');
        if (out === undefined) {
            ctx.fail('open-word.cli.out-required', { message: '`--out <file>` is required' });
            return 2;
        }
        const created = service.create({ title: text(options, 'title'), template: text(options, 'template') });
        try {
            const report = service.save(created.handle, out, flag(options, 'overwrite'));
            ctx.emitResult(
                { ...report, document: created.document },
                `created ${out} revision=${report.revision}`,
            );
            return 0;
        } finally {
            service.close(created.handle);
        }
    });

cli
    .command('open', 'cli.cmd.open')
    .actionCtx(async (options, _args, ctx) => {
        const file = input(options);
        if (file === undefined) {
            ctx.fail('open-word.cli.file-required', { message: 'a document file is required' });
            return 2;
        }
        const opened = service.open(file);
        try {
            ctx.emitResult(opened.document, describeDocument(opened.document));
            return 0;
        } finally {
            service.close(opened.handle);
        }
    });

cli
    .command('save', 'cli.cmd.save')
    .option('--out <file>', 'cli.opt.out')
    .option('--overwrite', 'cli.opt.overwrite')
    .actionCtx(async (options, _args, ctx) => {
        const file = input(options);
        if (file === undefined) {
            ctx.fail('open-word.cli.file-required', { message: 'a document file is required' });
            return 2;
        }
        const out = text(options, 'out');
        if (out === undefined) {
            ctx.fail('open-word.cli.out-required', { message: '`--out <file>` is required' });
            return 2;
        }
        const opened = service.open(file);
        try {
            const report = service.save(opened.handle, out, flag(options, 'overwrite'));
            ctx.emitResult(report, `saved ${out} revision=${report.revision} bytes=${report.bytes}`);
            return 0;
        } finally {
            service.close(opened.handle);
        }
    });

cli
    .command('inspect', 'cli.cmd.inspect')
    .option('--checks <list>', 'cli.opt.checks')
    .actionCtx(async (options, _args, ctx) => {
        const file = input(options);
        if (file === undefined) {
            ctx.fail('open-word.cli.file-required', { message: 'a document file is required' });
            return 2;
        }
        const checks = text(options, 'checks')
            ?.split(',')
            .map((name) => name.trim())
            .filter((name) => name.length > 0) as InspectCheck[] | undefined;
        const opened = service.open(file);
        try {
            const report = service.inspect(opened.handle, checks);
            ctx.emitResult(report, describeReport(report));
            return report.passed ? 0 : 1;
        } finally {
            service.close(opened.handle);
        }
    });

cli
    .command('edit', 'cli.cmd.edit')
    .option('--paragraph <text>', 'cli.opt.paragraph')
    .option('--heading <text>', 'cli.opt.heading')
    .option('--level <level>', 'cli.opt.level')
    .option('--table <json>', 'cli.opt.table')
    .option('--title <title>', 'cli.opt.title')
    .option('--author <author>', 'cli.opt.author')
    .option('--expected-revision <revision>', 'cli.opt.expectedRevision')
    .option('--out <file>', 'cli.opt.out')
    .option('--overwrite', 'cli.opt.overwrite')
    .actionCtx(async (options, _args, ctx) => {
        const file = input(options);
        if (file === undefined) {
            ctx.fail('open-word.cli.file-required', { message: 'a document file is required' });
            return 2;
        }

        const operations: EditOperation[] = [];
        const paragraph = text(options, 'paragraph');
        if (paragraph !== undefined) {
            operations.push({ type: 'paragraph.append', text: paragraph });
        }
        const heading = text(options, 'heading');
        if (heading !== undefined) {
            const level = parseInteger(text(options, 'level') ?? '1');
            if (level === undefined || level < 1 || level > 6) {
                ctx.fail('open-word.cli.edit-level', { message: '`--level <level>` must be between 1 and 6' });
                return 2;
            }
            operations.push({ type: 'heading.append', level, text: heading });
        }
        const table = text(options, 'table');
        if (table !== undefined) {
            const rows = parseRows(table);
            if (rows === undefined) {
                ctx.fail('open-word.cli.edit-table', { message: '`--table <json>` must be an array of string rows' });
                return 2;
            }
            operations.push({ type: 'table.append', rows });
        }
        const title = text(options, 'title');
        const author = text(options, 'author');
        if (title !== undefined || author !== undefined) {
            operations.push({ type: 'metadata.set', title, author });
        }
        if (operations.length === 0) {
            ctx.fail('open-word.cli.edit-args', { message: 'pass at least one of `--paragraph`, `--heading`, `--table`, `--title` or `--author`' });
            return 2;
        }

        const revisionText = text(options, 'expected-revision');
        const expectedRevision = revisionText === undefined ? undefined : parseInteger(revisionText);
        if (revisionText !== undefined && expectedRevision === undefined) {
            ctx.fail('open-word.cli.edit-revision', { message: '`--expected-revision` must be a non-negative integer' });
            return 2;
        }

        const opened = service.open(file);
        try {
            const report = service.edit(opened.handle, { operations, expectedRevision });
            const out = text(options, 'out');
            if (out !== undefined) {
                const saved = service.save(opened.handle, out, flag(options, 'overwrite'));
                ctx.emitResult({ ...report, saved }, `${describeDocument(report.document)}\n  saved: ${saved.path} (${saved.bytes} bytes)`);
            }
            else {
                ctx.emitResult(report, describeDocument(report.document));
            }
            return 0;
        } finally {
            service.close(opened.handle);
        }
    });

cli
    .command('export', 'cli.cmd.export')
    .option('--format <format>', 'cli.opt.format', { enum: ['docx', 'dotx', 'pdf', 'html', 'md'] })
    .option('--out <file>', 'cli.opt.out')
    .option('--overwrite', 'cli.opt.overwrite')
    .actionCtx(async (options, _args, ctx) => {
        const file = input(options);
        if (file === undefined) {
            ctx.fail('open-word.cli.file-required', { message: 'a document file is required' });
            return 2;
        }
        const format = text(options, 'format');
        const out = text(options, 'out');
        if (format === undefined || out === undefined) {
            ctx.fail('open-word.cli.export-args', { message: '`--format <format>` and `--out <file>` are required' });
            return 2;
        }
        const opened = service.open(file);
        try {
            const report = service.exportDocument(opened.handle, format as ExportFormat, out, flag(options, 'overwrite'));
            ctx.emitResult(report, describeExport(report));
            return 0;
        } finally {
            service.close(opened.handle);
        }
    });

cli.command('gui', 'cli.cmd.gui').actionCtx(async (_options, _args, ctx) => {
    try {
        const status = await runGui({});
        ctx.emitResult(status, `window ${status.state}`);
        return 0;
    } catch (error) {
        const message = error instanceof Error ? error.message : String(error);
        ctx.fail('open-word.cli.gui-failed', { message });
        return 1;
    }
});

const entry = process.argv[1];
if (entry !== undefined && import.meta.url === pathToFileURL(entry).href) {
    const code = await cli.parse(process.argv);
    process.exitCode = code;
}
