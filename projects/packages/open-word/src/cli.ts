#!/usr/bin/env node
/**
 * Command-line assembly only.
 *
 * Every command maps options onto one method of `new OpenWord()`; document
 * semantics, saving and export stay native. Help text and errors come from
 * `@wae/commander`, so the CLI does not grow its own parser.
 */

import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

import { createCli } from '@wae/commander';

import { OpenWord } from './open-word.js';
import { runGui } from './gui.js';
import type {
    Document,
    DocumentCapabilities,
    DocumentOutline,
    EditOperation,
    ExportFormat,
    ExportReport,
    FindHit,
    InspectReport,
    OutlineItem,
    SaveReport,
    StatsReport,
} from './types.js';
import type { InspectCheck } from './types.js';

const here = path.dirname(fileURLToPath(import.meta.url));

const cli = createCli('open-word').locales(path.join(here, '..', 'locales')).intro('cli.intro');

/** Runs `work` against one session and always releases its handles. */
async function withSession<T>(work: (word: OpenWord) => Promise<T>): Promise<T> {
    const word = new OpenWord();
    try {
        return await work(word);
    } finally {
        await word.dispose();
    }
}

// Exported for contract audits that import the built entry to inspect the
// command tree. Nothing runs until the file is executed as the program entry.
export { cli };

interface ParsedOptions {
    readonly _?: readonly string[];
    readonly [key: string]: unknown;
}

const text = (options: ParsedOptions, key: string): string | undefined =>
    typeof options[key] === 'string' ? (options[key] as string) : undefined;

const texts = (options: ParsedOptions, key: string): string[] => {
    const value = options[key];
    return Array.isArray(value) ? value.filter((entry): entry is string => typeof entry === 'string') : [];
};

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

function parseValues(value: string | undefined): Record<string, string | number | boolean> | undefined {
    if (value === undefined) {
        return undefined;
    }
    let parsed: unknown;
    try {
        parsed = JSON.parse(value);
    } catch {
        return undefined;
    }
    if (typeof parsed !== 'object' || parsed === null || Array.isArray(parsed)) {
        return undefined;
    }
    const entries = Object.entries(parsed as Record<string, unknown>);
    if (entries.length === 0) {
        return undefined;
    }
    const values: Record<string, string | number | boolean> = {};
    for (const [key, entry] of entries) {
        if (typeof entry !== 'string' && typeof entry !== 'number' && typeof entry !== 'boolean') {
            return undefined;
        }
        values[key] = entry;
    }
    return values;
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

function describeStats(report: StatsReport): string {
    const headings = Object.entries(report.headings)
        .filter(([, count]) => count > 0)
        .map(([level, count]) => `h${level}=${count}`)
        .join(', ');
    return [
        `revision=${report.revision} blocks=${report.blocks} paragraphs=${report.paragraphs} tables=${report.tables} rows=${report.tableRows}`,
        `headings: ${headings.length > 0 ? headings : 'none'}`,
        `words=${report.words} characters=${report.characters} (${report.charactersWithoutWhitespace} without spaces) reading=${report.readingMinutes} min`,
    ].join('\n');
}

function describeOutline(report: DocumentOutline): string {
    const lines = [`revision=${report.revision} headings=${countOutline(report.items)}`];
    const walk = (items: readonly OutlineItem[], depth: number): void => {
        for (const item of items) {
            const title = item.title ?? '(untitled)';
            lines.push(`${'  '.repeat(depth)}h${item.level} ${title} [${item.id} @${item.blockIndex}]`);
            walk(item.children, depth + 1);
        }
    };
    walk(report.items, 0);
    return lines.join('\n');
}

function countOutline(items: readonly OutlineItem[]): number {
    return items.reduce((total, item) => total + 1 + countOutline(item.children), 0);
}

function parseSteps(options: ParsedOptions): number | undefined | null {
    const value = text(options, 'steps');
    if (value === undefined) {
        return undefined;
    }
    const steps = parseInteger(value);
    return steps !== undefined && steps > 0 ? steps : null;
}

async function persistProject(file: string, word: OpenWord): Promise<SaveReport | undefined> {
    return path.extname(file).toLowerCase() === '.oword' ? word.save(file, { overwrite: true }) : undefined;
}

function describeHistory(saved: SaveReport | undefined): string {
    return saved === undefined
        ? '\n  history is session-local: a freshly opened document has an empty history'
        : `\n  saved: ${saved.path} (${saved.bytes} bytes)`;
}

function describeHits(hits: readonly FindHit[]): string {
    if (hits.length === 0) {
        return 'no matches';
    }
    return [
        `${hits.length} match(es)`,
        ...hits.map((hit) => `  ${hit.blockId} [${hit.start}, ${hit.end}) ${hit.text}`),
    ].join('\n');
}

cli.command('capabilities', 'cli.cmd.capabilities').actionCtx(async (_options, _args, ctx) => {
    const matrix = await withSession((word) => word.capabilities());
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
        await withSession(async (word) => {
            const document = await word.create({ title: text(options, 'title'), template: text(options, 'template') });
            const report = await word.save(out, { overwrite: flag(options, 'overwrite') });
            ctx.emitResult({ ...report, document }, `created ${out} revision=${report.revision}`);
        });
        return 0;
    });

cli
    .command('open', 'cli.cmd.open')
    .actionCtx(async (options, _args, ctx) => {
        const file = input(options);
        if (file === undefined) {
            ctx.fail('open-word.cli.file-required', { message: 'a document file is required' });
            return 2;
        }
        await withSession(async (word) => {
            const document = await word.open({ path: file });
            ctx.emitResult(document, describeDocument(document));
        });
        return 0;
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
        await withSession(async (word) => {
            await word.open({ path: file });
            const report = await word.save(out, { overwrite: flag(options, 'overwrite') });
            ctx.emitResult(report, `saved ${out} revision=${report.revision} bytes=${report.bytes}`);
        });
        return 0;
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
        let passed = false;
        await withSession(async (word) => {
            const report = await word.inspect({ path: file }, { checks });
            ctx.emitResult(report, describeReport(report));
            passed = report.passed;
        });
        return passed ? 0 : 1;
    });

cli
    .command('find', 'cli.cmd.find')
    .option('--limit <count>', 'cli.opt.limit')
    .actionCtx(async (options, _args, ctx) => {
        const file = input(options);
        const query = options._?.[1];
        if (file === undefined || query === undefined || query.length === 0) {
            ctx.fail('open-word.cli.find-args', { message: 'a document file and a non-empty query are required' });
            return 2;
        }
        const limitText = text(options, 'limit');
        const limit = limitText === undefined ? undefined : parseInteger(limitText);
        if (limitText !== undefined && (limit === undefined || limit < 1)) {
            ctx.fail('open-word.cli.find-limit', { message: '`--limit <count>` must be a positive integer' });
            return 2;
        }
        await withSession(async (word) => {
            const hits = await word.open({ path: file }).then(() => word.find(query, { limit }));
            ctx.emitResult(hits, describeHits(hits));
        });
        return 0;
    });

cli.command('stats', 'cli.cmd.stats').actionCtx(async (options, _args, ctx) => {
    const file = input(options);
    if (file === undefined) {
        ctx.fail('open-word.cli.file-required', { message: 'a document file is required' });
        return 2;
    }
    await withSession(async (word) => {
        await word.open({ path: file });
        const report = await word.stats();
        ctx.emitResult(report, describeStats(report));
    });
    return 0;
});

cli.command('outline', 'cli.cmd.outline').actionCtx(async (options, _args, ctx) => {
    const file = input(options);
    if (file === undefined) {
        ctx.fail('open-word.cli.file-required', { message: 'a document file is required' });
        return 2;
    }
    await withSession(async (word) => {
        await word.open({ path: file });
        const report = await word.outline();
        ctx.emitResult(report, describeOutline(report));
    });
    return 0;
});

cli
    .command('edit', 'cli.cmd.edit')
    .option('--paragraph <text>', 'cli.opt.paragraph')
    .option('--heading <text>', 'cli.opt.heading')
    .option('--heading-id <id>', 'cli.opt.headingId')
    .option('--level <level>', 'cli.opt.level')
    .option('--bullet <text>...', 'cli.opt.bullet')
    .option('--numbered <text>...', 'cli.opt.numbered')
    .option('--numbered-start <n>', 'cli.opt.numberedStart')
    .option('--task <text>...', 'cli.opt.task')
    .option('--task-done <text>...', 'cli.opt.taskDone')
    .option('--quote <text>', 'cli.opt.quote')
    .option('--code <text>', 'cli.opt.code')
    .option('--code-lang <lang>', 'cli.opt.codeLang')
    .option('--table <json>', 'cli.opt.table')
    .option('--duplicate-block <id>', 'cli.opt.duplicateBlock')
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
        const headingId = text(options, 'heading-id');
        if (headingId !== undefined) {
            const levelText = text(options, 'level');
            if (levelText === undefined) {
                ctx.fail('open-word.cli.edit-level', { message: '`--heading-id` requires `--level <1..6>`' });
                return 2;
            }
            const level = parseInteger(levelText);
            if (level === undefined || level < 1 || level > 6) {
                ctx.fail('open-word.cli.edit-level', { message: '`--level <level>` must be between 1 and 6' });
                return 2;
            }
            operations.push({ type: 'heading.setLevel', id: headingId, level });
        }
        const bullets = texts(options, 'bullet');
        if (bullets.length > 0) {
            operations.push({ type: 'list.append', ordered: false, items: bullets });
        }
        const numbered = texts(options, 'numbered');
        const numberedStartText = text(options, 'numbered-start');
        const numberedStart = numberedStartText === undefined ? undefined : parseInteger(numberedStartText);
        if (numberedStartText !== undefined && (numberedStart === undefined || numberedStart < 1)) {
            ctx.fail('open-word.cli.edit-numbered-start', { message: '`--numbered-start <n>` must be a positive integer' });
            return 2;
        }
        if (numberedStartText !== undefined && numbered.length === 0) {
            ctx.fail('open-word.cli.edit-numbered-start', { message: '`--numbered-start <n>` requires `--numbered`' });
            return 2;
        }
        if (numbered.length > 0) {
            operations.push({
                type: 'list.append',
                ordered: true,
                items: numbered,
                ...(numberedStart === undefined ? {} : { start: numberedStart }),
            });
        }
        const tasks = [
            ...texts(options, 'task').map((text) => ({ text })),
            ...texts(options, 'task-done').map((text) => ({ text, checked: true })),
        ];
        if (tasks.length > 0) {
            operations.push({ type: 'taskList.append', items: tasks });
        }
        const quote = text(options, 'quote');
        if (quote !== undefined) {
            operations.push({ type: 'quote.append', text: quote });
        }
        const code = text(options, 'code');
        const codeLang = text(options, 'code-lang');
        if (codeLang !== undefined && code === undefined) {
            ctx.fail('open-word.cli.edit-code-lang', { message: '`--code-lang <lang>` requires `--code`' });
            return 2;
        }
        if (code !== undefined) {
            operations.push({ type: 'code.append', text: code, ...(codeLang === undefined ? {} : { language: codeLang }) });
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
        const duplicateBlock = text(options, 'duplicate-block');
        if (duplicateBlock !== undefined) {
            operations.push({ type: 'block.duplicate', id: duplicateBlock });
        }
        const title = text(options, 'title');
        const author = text(options, 'author');
        if (title !== undefined || author !== undefined) {
            operations.push({ type: 'metadata.set', title, author });
        }
        if (operations.length === 0) {
            ctx.fail('open-word.cli.edit-args', {
                message:
                    'pass at least one of `--paragraph`, `--heading`, `--heading-id`, `--bullet`, `--numbered`, `--quote`, `--code`, `--table`, `--duplicate-block`, `--title` or `--author`',
            });
            return 2;
        }

        const revisionText = text(options, 'expected-revision');
        const expectedRevision = revisionText === undefined ? undefined : parseInteger(revisionText);
        if (revisionText !== undefined && expectedRevision === undefined) {
            ctx.fail('open-word.cli.edit-revision', { message: '`--expected-revision` must be a non-negative integer' });
            return 2;
        }

        await withSession(async (word) => {
            await word.open({ path: file });
            const report = await word.edit(operations, { expectedRevision });
            const out = text(options, 'out');
            if (out !== undefined) {
                const saved = await word.save(out, { overwrite: flag(options, 'overwrite') });
                ctx.emitResult({ ...report, saved }, `${describeDocument(report.document)}\n  saved: ${saved.path} (${saved.bytes} bytes)`);
            }
            else {
                ctx.emitResult(report, describeDocument(report.document));
            }
        });
        return 0;
    });

cli
    .command('replace', 'cli.cmd.replace')
    .option('--find <text>', 'cli.opt.find')
    .option('--with <text>', 'cli.opt.with')
    .option('--ignore-case', 'cli.opt.ignoreCase')
    .option('--whole-word', 'cli.opt.wholeWord')
    .option('--regex', 'cli.opt.regex')
    .option('--out <file>', 'cli.opt.out')
    .option('--overwrite', 'cli.opt.overwrite')
    .actionCtx(async (options, _args, ctx) => {
        const file = input(options);
        if (file === undefined) {
            ctx.fail('open-word.cli.file-required', { message: 'a document file is required' });
            return 2;
        }
        const find = text(options, 'find');
        const replacement = text(options, 'with');
        if (find === undefined || find.length === 0 || replacement === undefined) {
            ctx.fail('open-word.cli.replace-args', { message: '`--find <text>` and `--with <text>` are required' });
            return 2;
        }
        await withSession(async (word) => {
            await word.open({ path: file });
            const report = await word.replace(find, replacement, {
                caseSensitive: !flag(options, 'ignore-case'),
                wholeWord: flag(options, 'whole-word'),
                regex: flag(options, 'regex'),
            });
            const replaced = report.replaced;
            const out = text(options, 'out');
            if (out !== undefined) {
                const saved = await word.save(out, { overwrite: flag(options, 'overwrite') });
                ctx.emitResult(
                    { ...report, saved },
                    `${describeDocument(report.document)}\n  replaced: ${replaced}\n  saved: ${saved.path} (${saved.bytes} bytes)`,
                );
            }
            else {
                ctx.emitResult(report, `${describeDocument(report.document)}\n  replaced: ${replaced}`);
            }
        });
        return 0;
    });

cli
    .command('template', 'cli.cmd.template')
    .option('--values <json>', 'cli.opt.values')
    .option('--out <file>', 'cli.opt.out')
    .option('--overwrite', 'cli.opt.overwrite')
    .actionCtx(async (options, _args, ctx) => {
        const file = input(options);
        if (file === undefined) {
            ctx.fail('open-word.cli.file-required', { message: 'a document file is required' });
            return 2;
        }
        const values = parseValues(text(options, 'values'));
        if (values === undefined) {
            ctx.fail('open-word.cli.template-values', {
                message: '`--values <json>` must be a non-empty JSON object of string, number or boolean values',
            });
            return 2;
        }
        const out = text(options, 'out');
        if (out === undefined) {
            ctx.fail('open-word.cli.out-required', { message: '`--out <file>` is required' });
            return 2;
        }
        await withSession(async (word) => {
            await word.open({ path: file });
            const report = await word.edit([{ type: 'template.fill', values }]);
            const replaced = report.results[0]?.replaced ?? 0;
            const saved = await word.save(out, { overwrite: flag(options, 'overwrite') });
            ctx.emitResult(
                { ...report, saved },
                `${describeDocument(report.document)}\n  replaced: ${replaced}\n  saved: ${saved.path} (${saved.bytes} bytes)`,
            );
        });
        return 0;
    });

for (const action of ['undo', 'redo'] as const) {
    cli
        .command(action, `cli.cmd.${action}`)
        .option('--steps <count>', 'cli.opt.steps')
        .actionCtx(async (options, _args, ctx) => {
            const file = input(options);
            if (file === undefined) {
                ctx.fail('open-word.cli.file-required', { message: 'a document file is required' });
                return 2;
            }
            const steps = parseSteps(options);
            if (steps === null) {
                ctx.fail('open-word.cli.history-steps', { message: '`--steps <count>` must be a positive integer' });
                return 2;
            }
            await withSession(async (word) => {
                await word.open({ path: file });
                const result = action === 'undo' ? await word.undo({ steps }) : await word.redo({ steps });
                const saved = await persistProject(file, word);
                const count = 'undone' in result ? result.undone : result.redone;
                const availability =
                    'undone' in result
                        ? `redoAvailable=${result.redoAvailable}`
                        : `undoAvailable=${result.undoAvailable}`;
                ctx.emitResult(
                    { ...result, saved },
                    `${action} ${count} step(s) -> revision=${result.revision} ${availability}\n${describeDocument(result.document)}${describeHistory(saved)}`,
                );
            });
            return 0;
        });
}

cli.command('merge', 'cli.cmd.merge').actionCtx(async (options, _args, ctx) => {
    const file = input(options);
    const other = options._?.[1];
    if (file === undefined || other === undefined || other.length === 0) {
        ctx.fail('open-word.cli.merge-args', { message: 'a document file and another document are required' });
        return 2;
    }
    await withSession(async (word) => {
        await word.open({ path: file });
        const report = await word.merge(other);
        const saved = await persistProject(file, word);
        ctx.emitResult(
            { ...report, saved },
            `merged ${report.blocksAdded} block(s) from ${other}\n${describeDocument(report.summary)}\n  losses: ${report.losses.length}${describeHistory(saved)}`,
        );
    });
    return 0;
});

cli
    .command('export', 'cli.cmd.export')
    .option('--format <format>', 'cli.opt.format', { enum: ['docx', 'dotx', 'pdf', 'html', 'md', 'txt', 'rtf'] })
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
        await withSession(async (word) => {
            await word.open({ path: file });
            const report = await word.export({
                format: format as ExportFormat,
                target: out,
                overwrite: flag(options, 'overwrite'),
            });
            ctx.emitResult(report, describeExport(report));
        });
        return 0;
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
