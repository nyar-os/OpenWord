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
import test from 'node:test';

import { OpenWord, OpenWordError, documentFormats, formatFromPath, formatInfo } from './index.js';
import type { DocumentOpenInput, ExportFormat } from './index.js';
import { nativeBinding } from './native.js';

const installed = (() => {
    try {
        return nativeBinding() !== undefined;
    } catch {
        return false;
    }
})();
const withAddon = installed ? 'a built addon is installed' : false;

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
        (error: unknown) => error instanceof OpenWordError && error.code === 'format/unsupported',
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
