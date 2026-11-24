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
