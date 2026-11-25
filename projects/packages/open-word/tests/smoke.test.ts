/**
 * Smoke tests for the authoring shell.
 *
 * They assert shell semantics only: the stable error code without a built
 * binding, a session that closes idempotently, synchronous argument validation
 * for create/open/edit/save/export/find, and a format registry that classifies
 * the whole family. Document behaviour is covered by the engine's own round-trip
 * tests, not here.
 */

import assert from 'node:assert/strict';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';

import { OpenWord, OpenWordError, documentFormats, formatFromPath, formatInfo } from '../src/index.js';
import type { DocumentOpenInput, ExportFormat } from '../src/index.js';
import { nativeBinding } from '../src/native.js';

const installed = (() => {
    try {
        return nativeBinding() !== undefined;
    } catch {
        return false;
    }
})();
const withAddon = installed ? 'a built addon is installed' : false;
const withoutAddon = installed ? false : 'a built addon is not installed';

test('session construction is synchronous and side-effect free', async () => {
    const word = new OpenWord();
    assert.equal(word.locale, 'en');
    assert.equal(word.disposed, false);
    await word.dispose();
    assert.equal(word.disposed, true);
});

test('without an installed addon engine calls keep capability/not-implemented', { skip: withAddon }, async () => {
    const word = new OpenWord({ locale: 'zh-CN' });
    assert.equal(word.locale, 'zh-CN');
    const calls: (() => Promise<unknown>)[] = [
        () => word.capabilities(),
        () => word.create({ title: 'Research review' }),
        () => word.open({ path: 'report.docx' }),
        () => word.inspect({ path: 'report.docx' }),
    ];
    for (const call of calls) {
        await assert.rejects(call, (error: unknown) => {
            assert.ok(error instanceof OpenWordError);
            assert.equal(error.code, 'capability/not-implemented');
            assert.ok(error.details.feature);
            return true;
        });
    }
    await word.dispose();
});

test('dispose is idempotent and closes the session', async () => {
    const word = new OpenWord();
    await word.dispose();
    await word.dispose();
    await assert.rejects(word.capabilities(), (error: unknown) => error instanceof OpenWordError && error.code === 'session/disposed');
});

test('dispose after a rejected create and open stays clean and idempotent', { skip: withAddon }, async () => {
    const word = new OpenWord();
    await assert.rejects(word.create({ title: 'Rejected' }), (error: unknown) => error instanceof OpenWordError);
    await assert.rejects(word.open({ path: 'report.docx' }), (error: unknown) => error instanceof OpenWordError);
    await word.dispose();
    await word.dispose();
    await assert.rejects(
        word.capabilities(),
        (error: unknown) => error instanceof OpenWordError && error.code === 'session/disposed',
    );
});

test('open rejects a missing path before reaching the engine', async () => {
    const word = new OpenWord();
    const inputs = [{} as unknown as DocumentOpenInput, { path: '' }, { path: 42 as unknown as string }];
    for (const input of inputs) {
        await assert.rejects(
            word.open(input),
            (error: unknown) => error instanceof OpenWordError && error.code === 'session/argument-invalid',
        );
    }
    await word.dispose();
});

test('open rejects a non-editing intent before reaching the engine', async () => {
    const word = new OpenWord();
    await assert.rejects(
        word.open({ path: 'template.dotx', intent: 'template' }),
        (error: unknown) => error instanceof OpenWordError && error.code === 'capability/not-implemented',
    );
    await word.dispose();
});

test('edit, save and export validate arguments before the engine is reached', async () => {
    const word = new OpenWord();
    const invalid: (() => Promise<unknown>)[] = [
        () => word.save(''),
        () => word.export({ format: '' as unknown as ExportFormat, target: 'report.docx' }),
        () => word.export({ format: 'docx', target: '' }),
        () => word.edit([]),
        () => word.edit([{ type: 42 } as unknown as { readonly type: 'paragraph.append'; readonly text: string }]),
        () => word.edit([{ type: 'paragraph.append', text: 'x' }], { expectedRevision: -1 }),
    ];
    for (const call of invalid) {
        await assert.rejects(
            call,
            (error: unknown) => error instanceof OpenWordError && error.code === 'session/argument-invalid',
        );
    }
    await word.dispose();
});

test('edit, save and export need an open document', async () => {
    const word = new OpenWord();
    const calls: (() => Promise<unknown>)[] = [
        () => word.save('report.oword'),
        () => word.export({ format: 'docx', target: 'report.docx' }),
        () => word.edit([{ type: 'paragraph.append', text: 'x' }]),
    ];
    for (const call of calls) {
        await assert.rejects(
            call,
            (error: unknown) => error instanceof OpenWordError && error.code === 'document/not-found',
        );
    }
    await word.dispose();
});

test('find validates the query and limit before the engine is reached', async () => {
    const word = new OpenWord();
    const invalid: (() => Promise<unknown>)[] = [
        () => word.find(''),
        () => word.find(42 as unknown as string),
        () => word.find('alpha', { limit: 0 }),
        () => word.find('alpha', { limit: -1 }),
        () => word.find('alpha', { limit: 1.5 }),
    ];
    for (const call of invalid) {
        await assert.rejects(
            call,
            (error: unknown) => error instanceof OpenWordError && error.code === 'session/argument-invalid',
        );
    }
    await word.dispose();
});

test('find needs an open document', async () => {
    const word = new OpenWord();
    await assert.rejects(
        word.find('alpha'),
        (error: unknown) => error instanceof OpenWordError && error.code === 'document/not-found',
    );
    await word.dispose();
});

test('outline nests headings by level in an open document', { skip: withoutAddon }, async () => {
    const word = new OpenWord();
    await word.create({ title: 'Outline' });
    await word.edit([
        { type: 'heading.append', level: 1, text: 'Overview' },
        { type: 'heading.append', level: 2, text: 'Details' },
        { type: 'heading.append', level: 1, text: 'Appendix' },
    ]);
    const report = await word.outline();
    assert.equal(report.revision, 3);
    assert.equal(report.items.length, 2);
    assert.equal(report.items[0].title, 'Overview');
    assert.equal(report.items[0].level, 1);
    assert.ok(report.items[0].id.startsWith('section-'));
    assert.equal(report.items[0].children.length, 1);
    assert.equal(report.items[0].children[0].title, 'Details');
    assert.equal(report.items[0].children[0].level, 2);
    assert.deepEqual(report.items[0].children[0].children, []);
    assert.equal(report.items[1].title, 'Appendix');
    assert.deepEqual(report.items[1].children, []);
    await word.dispose();
});

test('replace validates the request before the engine is reached', async () => {
    const word = new OpenWord();
    const invalid: (() => Promise<unknown>)[] = [
        () => word.replace('', 'x'),
        () => word.replace(42 as unknown as string, 'x'),
        () => word.replace('alpha', 42 as unknown as string),
        () => word.replace('alpha', 'x', { caseSensitive: 'yes' as unknown as boolean }),
        () => word.replace('alpha', 'x', { wholeWord: 1 as unknown as boolean }),
        () => word.replace('alpha', 'x', { limit: 0 }),
        () => word.replace('alpha', 'x', { expectedRevision: -1 }),
    ];
    for (const call of invalid) {
        await assert.rejects(
            call,
            (error: unknown) => error instanceof OpenWordError && error.code === 'session/argument-invalid',
        );
    }
    await word.dispose();
});

test('replace needs an open document', async () => {
    const word = new OpenWord();
    await assert.rejects(
        word.replace('alpha', 'x'),
        (error: unknown) => error instanceof OpenWordError && error.code === 'document/not-found',
    );
    await word.dispose();
});

test('undo, redo and merge validate arguments before the engine is reached', async () => {
    const word = new OpenWord();
    const invalid: (() => Promise<unknown>)[] = [
        () => word.undo({ steps: 0 }),
        () => word.undo({ steps: -1 }),
        () => word.redo({ steps: 1.5 }),
        () => word.merge(''),
        () => word.merge(42 as unknown as string),
    ];
    for (const call of invalid) {
        await assert.rejects(
            call,
            (error: unknown) => error instanceof OpenWordError && error.code === 'session/argument-invalid',
        );
    }
    await word.dispose();
});

test('undo, redo and merge need an open document', async () => {
    const word = new OpenWord();
    const calls: (() => Promise<unknown>)[] = [
        () => word.undo(),
        () => word.redo({ steps: 2 }),
        () => word.merge('other.md'),
    ];
    for (const call of calls) {
        await assert.rejects(
            call,
            (error: unknown) => error instanceof OpenWordError && error.code === 'document/not-found',
        );
    }
    await word.dispose();
});

test('format registry classifies the document format family', () => {
    const expected = ['docx', 'docm', 'dotx', 'dotm', 'doc', 'rtf', 'odt', 'txt', 'md', 'html', 'pdf'];
    assert.deepEqual([...documentFormats].sort(), [...expected].sort());

    assert.equal(formatInfo('dotx').defaultIntent, 'template');
    assert.equal(formatInfo('dotm').defaultIntent, 'template');
    assert.equal(formatInfo('dotm').macroEnabled, true);
    assert.equal(formatInfo('docm').macroEnabled, true);
    assert.equal(formatInfo('doc').opensAs, 'none');
    assert.equal(formatInfo('doc').defaultIntent, null);
    assert.equal(formatInfo('rtf').defaultIntent, 'read');
    assert.equal(formatInfo('odt').family, 'interchange');
    assert.equal(formatInfo('pdf').opensAs, 'none');
    assert.equal(formatInfo('pdf').defaultIntent, null);

    assert.equal(formatFromPath('Report.DOCX'), 'docx');
    assert.equal(formatFromPath('report.oword'), undefined);
    assert.equal(formatFromPath('notes.key'), undefined);
    assert.equal(formatFromPath('no-extension'), undefined);
});

test('real binding failures carry the typed params of their code', { skip: withoutAddon }, async () => {
    const word = new OpenWord();
    try {
        await assert.rejects(
            word.open({ path: join(tmpdir(), 'open-word-missing-file.docx') }),
            (error: unknown) => {
                assert.ok(error instanceof OpenWordError);
                const failure = error.failure;
                assert.equal(failure.code, 'io/read');
                if (failure.code !== 'io/read') return false;
                assert.equal(failure.params.kind, 'NotFound');
                assert.equal(typeof failure.params.path, 'string');
                return true;
            },
        );

        await word.create({ title: 'Params' });
        await assert.rejects(
            word.edit([{ type: 'block.duplicate', id: 'node-999999' }]),
            (error: unknown) => {
                assert.ok(error instanceof OpenWordError);
                const failure = error.failure;
                assert.equal(failure.code, 'object/not-found');
                if (failure.code !== 'object/not-found') return false;
                assert.equal(failure.params.kind, 'block');
                return true;
            },
        );
    } finally {
        await word.dispose();
    }
});
