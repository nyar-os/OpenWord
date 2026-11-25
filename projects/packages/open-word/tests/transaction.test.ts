/**
 * Mixed-transaction and rollback cases for the SDK edit batch.
 *
 * One batch mixes block, list and table operations and must apply as a unit;
 * a batch whose last operation is refused must leave the committed document
 * untouched — same revision, same content. These run against the real binding
 * and skip cleanly when no addon is installed.
 */

import assert from 'node:assert/strict';
import test from 'node:test';

import { OpenWord, OpenWordError } from '../src/index.js';
import { nativeBinding } from '../src/native.js';

const withAddon = nativeBinding() !== undefined ? undefined : 'a built addon is not installed';

test('a mixed block, list and table batch applies atomically', { skip: withAddon }, async () => {
    const word = new OpenWord();
    await word.create({ title: 'Ledger' });
    const base = await word.edit([
        { type: 'heading.append', level: 1, text: 'Plan' },
        { type: 'paragraph.append', text: 'Anchor paragraph.' },
        { type: 'list.append', items: ['One', 'Two'] },
        { type: 'table.append', rows: [['a', 'b'], ['c', 'd']] },
    ]);
    const revision = base.revision;

    const mixed = await word.edit([
        { type: 'block.duplicate', id: 'section-1' },
        { type: 'list.setItem', id: 'node-3', item: 0, text: 'Uno' },
        { type: 'table.setCell', table: 0, row: 1, column: 1, text: 'dd' },
    ]);
    assert.equal(mixed.results.length, 3);
    assert.equal(mixed.revision, revision + 3, 'each applied operation advances the revision once');
    assert.equal((await word.find('Uno')).length, 1);
    assert.equal((await word.find('dd')).length, 1);
    assert.equal((await word.find('Plan')).length, 2, 'the duplicated heading joins the document');
    await word.dispose();
});

test('a refused tail rolls the whole batch back', { skip: withAddon }, async () => {
    const word = new OpenWord();
    await word.create({ title: 'Ledger' });
    const base = await word.edit([
        { type: 'paragraph.append', text: 'Kept.' },
        { type: 'list.append', items: ['Only'] },
        { type: 'table.append', rows: [['x']] },
    ]);

    await assert.rejects(
        word.edit([
            { type: 'paragraph.append', text: 'Never lands.' },
            { type: 'block.duplicate', id: 'node-1' },
            { type: 'table.setCell', table: 9, row: 0, column: 0, text: 'nope' },
        ]),
        (error: unknown) => {
            assert.ok(error instanceof OpenWordError);
            assert.equal(error.code, 'session/argument-invalid');
            return true;
        },
    );

    const after = await word.stats();
    assert.equal(after.revision, base.revision, 'a rejected batch never advances the revision');
    assert.equal((await word.find('Never lands.')).length, 0);
    await word.dispose();
});

test('a stale expectedRevision rejects without touching the document', { skip: withAddon }, async () => {
    const word = new OpenWord();
    const created = await word.create({ title: 'Ledger' });
    const base = await word.edit([{ type: 'paragraph.append', text: 'First.' }]);
    assert.equal(base.revision, created.revision + 1);

    await assert.rejects(
        word.edit([{ type: 'paragraph.append', text: 'Stale.' }], { expectedRevision: created.revision }),
        (error: unknown) => {
            assert.ok(error instanceof OpenWordError);
            assert.equal(error.code, 'document/revision-conflict');
            return true;
        },
    );
    const after = await word.stats();
    assert.equal(after.revision, base.revision);
    assert.equal((await word.find('Stale.')).length, 0);
    await word.dispose();
});

test('a batch of convert, swap, reverse and transpose transforms applies atomically', { skip: withAddon }, async () => {
    const word = new OpenWord();
    await word.create({ title: 'Ledger' });
    const base = await word.edit([
        { type: 'paragraph.append', text: 'Convert me.' },
        { type: 'paragraph.append', text: 'Swap partner.' },
        { type: 'heading.append', level: 1, text: 'Alpha' },
        { type: 'list.append', items: ['One', 'Two', 'Three'], ordered: true },
        { type: 'table.append', rows: [['a', 'b', 'c'], ['d', 'e', 'f']] },
    ]);

    const mixed = await word.edit([
        { type: 'block.convert', id: 'node-1', to: 'quote' },
        { type: 'block.swap', id: 'node-1', target: 'node-2' },
        { type: 'list.reverseItems', id: 'node-4' },
        { type: 'table.transpose', table: 0 },
        { type: 'list.setItem', id: 'node-4', item: 0, text: 'Zed' },
    ]);
    assert.equal(mixed.results.length, 5);
    assert.equal(mixed.revision, base.revision + 5, 'each transform advances the revision once');
    assert.deepEqual(mixed.results[0], { type: 'block.convert', id: 'node-1', from: 'paragraph', to: 'quote' });
    assert.deepEqual(mixed.results[2], { type: 'list.reverseItems', id: 'node-4', items: 3 });
    assert.deepEqual(mixed.results[3], { type: 'table.transpose', table: 0, rows: 3, columns: 2, headRows: 1, footRows: 0 });
    assert.deepEqual(mixed.document.tables, [{ index: 0, sectionIndex: 0, rows: 3, columns: 2 }], 'the 2x3 table transposed to 3x2');
    assert.equal((await word.find('Swap partner.'))[0].blockIndex, 0, 'the swapped paragraph leads the document');
    assert.equal((await word.find('Three')).length, 0, 'the reversed list lost its old first item');
    assert.equal((await word.find('Zed')).length, 1, 'the list tail item landed');
    await word.dispose();
});

test('an unknown swap target rolls the whole transform batch back', { skip: withAddon }, async () => {
    const word = new OpenWord();
    await word.create({ title: 'Ledger' });
    const base = await word.edit([
        { type: 'paragraph.append', text: 'Convert me.' },
        { type: 'table.append', rows: [['a', 'b', 'c'], ['d', 'e', 'f']] },
    ]);

    await assert.rejects(
        word.edit([
            { type: 'table.transpose', table: 0 },
            { type: 'block.swap', id: 'node-1', target: 'node-999999' },
        ]),
        (error: unknown) => {
            assert.ok(error instanceof OpenWordError);
            const failure = error.failure;
            assert.equal(failure.code, 'object/not-found');
            if (failure.code !== 'object/not-found') return false;
            assert.equal(failure.params.kind, 'block');
            return true;
        },
    );

    const after = await word.stats();
    assert.equal(after.revision, base.revision, 'a rejected transform batch never advances the revision');
    const probe = await word.edit([{ type: 'paragraph.append', text: 'Still 2x3.' }]);
    assert.equal(probe.document.tables.length, 1);
    assert.equal(probe.document.tables[0].rows, 2, 'the transposed table rolled back');
    assert.equal(probe.document.tables[0].columns, 3);
    assert.equal((await word.find('Convert me.')).length, 1);
    await word.dispose();
});

test('a stale expectedRevision rejects a transform batch without applying any operation', { skip: withAddon }, async () => {
    const word = new OpenWord();
    const created = await word.create({ title: 'Ledger' });
    const base = await word.edit([
        { type: 'list.append', items: ['One', 'Two', 'Three'], ordered: true },
        { type: 'table.append', rows: [['a', 'b', 'c'], ['d', 'e', 'f']] },
    ]);
    assert.equal(base.revision, created.revision + 2);

    await assert.rejects(
        word.edit(
            [
                { type: 'list.reverseItems', id: 'node-1' },
                { type: 'table.transpose', table: 0 },
            ],
            { expectedRevision: created.revision },
        ),
        (error: unknown) => {
            assert.ok(error instanceof OpenWordError);
            const failure = error.failure;
            assert.equal(failure.code, 'document/revision-conflict');
            if (failure.code !== 'document/revision-conflict') return false;
            assert.equal(failure.params.expected, created.revision);
            return true;
        },
    );

    const after = await word.stats();
    assert.equal(after.revision, base.revision, 'a stale batch never advances the revision');
    const probe = await word.edit([{ type: 'paragraph.append', text: 'Probe.' }]);
    assert.equal(probe.document.tables.length, 1);
    assert.equal(probe.document.tables[0].rows, 2, 'the transpose in the stale batch never applied');
    assert.equal(probe.document.tables[0].columns, 3);
    await word.dispose();
});
