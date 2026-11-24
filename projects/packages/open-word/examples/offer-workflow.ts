/**
 * End-to-end offer workflow, the runnable form of `documentation/agents.md`.
 *
 * The script generates its own DOCX template fixture, fills it through the
 * SDK, delivers DOCX, a review PDF and the native project, then reopens every
 * artifact and asserts the delivered content: placeholders are gone, the table
 * keeps its grid, header band and merged cell, the footer row's content stays
 * in the table, the ordered and unordered lists keep their markers, the nested
 * ordered list keeps its nesting in the native project and the PDF really
 * paginates. It exits non-zero on the first failed assertion, so CI runs the
 * documented workflow verbatim instead of trusting the prose.
 *
 * Run from the repository root with a built binding installed or staged:
 *
 * ```bash
 * pnpm --filter @wae/open-word run build:test
 * node projects/packages/open-word/dist-test/examples/offer-workflow.js
 * ```
 *
 * On hosts without an installed platform package point `OPEN_WORD_NATIVE` at a
 * freshly built addon (`target/debug/open_word_napi.dll` or `.so`).
 */

import assert from 'node:assert/strict';
import { mkdtempSync, readFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

import { OpenWord } from '../src/index.js';
import { nativeBinding } from '../src/native.js';

type Json = Record<string, any>;

/** Reads the native `.oword` IR envelope, the engine's authoritative structure dump. */
function readEnvelope(path: string): Json {
    return JSON.parse(readFileSync(path, 'utf8')) as Json;
}

function blocksOf(envelope: Json): Json[] {
    return (envelope.document.blocks as Json[]).map((block) => block.block as Json);
}

function tableOf(envelope: Json): Json {
    const block = blocksOf(envelope).find((entry) => entry.Table !== undefined);
    assert.ok(block !== undefined, 'the envelope holds the delivered table');
    return block.Table as Json;
}

function listOf(envelope: Json): Json[] {
    return blocksOf(envelope).filter((block) => block.List !== undefined).map((block) => block.List as Json);
}

function allCells(table: Json): Json[] {
    return (['head', 'body', 'foot'] as const).flatMap((band) => (table[band] as Json[] | undefined) ?? [])
        .flatMap((row) => row.cells as Json[]);
}

function cellText(envelope: Json, cell: Json): string {
    const ids = cell.children as number[];
    return ids
        .map((id) => (envelope.document.blocks as Json[]).find((block) => block.id === id)?.block?.Paragraph)
        .map((paragraph) => (paragraph ? (paragraph.content as Json[]).map((inline) => inline.Text?.text ?? '').join('') : ''))
        .join('');
}

function itemText(item: Json): string {
    return (item.content as Json[]).map((inline) => inline.Text?.text ?? '').join('');
}

async function main(): Promise<void> {
    if (nativeBinding() === undefined) {
        throw new Error('this example needs a built native binding; install the platform package or set OPEN_WORD_NATIVE');
    }
    const dir = mkdtempSync(join(tmpdir(), 'open-word-offer-'));
    const templatePath = join(dir, 'offer-template.docx');
    const docxPath = join(dir, 'offer.docx');
    const pdfPath = join(dir, 'offer-review.pdf');
    const projectPath = join(dir, 'offer.oword');

    // 1. Generate the template fixture: placeholders, lists and a table that
    // exercises the semantics the reopen step must find intact — a merged
    // header cell, a footer row, an unordered list and a nested ordered list.
    const author = new OpenWord();
    await author.create({ title: 'Offer' });
    await author.edit([
        { type: 'heading.append', level: 1, text: 'Offer for {{customer}}' },
        { type: 'paragraph.append', text: 'Total {{amount}} due {{due}}. DRAFT terms apply.' },
        { type: 'list.append', items: ['Scope locked', 'DRAFT watermark'] },
        { type: 'list.append', items: ['Milestone A', 'Milestone B'], ordered: true },
        { type: 'list.indentItem', id: 'node-4', item: 1 },
        { type: 'table.append', rows: [['Field', 'Value'], ['Customer', '{{customer}}'], ['Amount', '{{amount}}']] },
        { type: 'table.setCellSpan', table: 0, row: 0, column: 0, colspan: 2, rowspan: 1 },
        { type: 'table.setFooter', table: 0, cells: ['Due', 'Net 30'] },
    ]);
    const template = await author.export({ format: 'docx', target: templatePath, overwrite: true });
    assert.equal(template.editable, true);
    await author.dispose();

    // 2. Fill business data, normalise wording and deliver in three formats.
    const word = new OpenWord();
    const opened = await word.open({ path: templatePath });
    const filled = await word.edit([
        { type: 'template.fill', values: { customer: 'ACME', amount: '1,200.00', due: '2026-12-01' } },
        { type: 'paragraph.append', text: 'Payment is due within 30 days.' },
    ]);
    assert.equal(filled.revision, opened.revision + 2, 'each applied operation advances the revision once');
    const replaced = await word.replace('DRAFT', 'FINAL', { wholeWord: true });
    assert.ok(replaced.replaced >= 2, 'the body paragraph and the list item both normalise');

    const docx = await word.export({ format: 'docx', target: docxPath, overwrite: true });
    const pdf = await word.export({ format: 'pdf', target: pdfPath, overwrite: true });
    const saved = await word.save(projectPath, { overwrite: true });
    assert.equal(docx.revision, replaced.revision);
    assert.equal(pdf.revision, replaced.revision);
    assert.equal(saved.revision, replaced.revision);
    assert.equal(pdf.editable, false, 'the PDF is a projection');
    await word.dispose();

    // 3. Reopen the delivered DOCX and verify the content, not just the bytes.
    const delivery = new OpenWord();
    const reopened = await delivery.open({ path: docxPath });
    assert.equal((await delivery.find('{{')).length, 0, 'no placeholder survives the fill');
    assert.ok((await delivery.find('ACME')).length >= 2, 'the customer reaches the body and the table');
    assert.ok((await delivery.find('1,200.00')).length >= 2);
    assert.equal((await delivery.find('2026-12-01')).length, 1);
    assert.equal((await delivery.find('DRAFT')).length, 0, 'the whole-word replacement landed');
    assert.ok((await delivery.find('FINAL')).length >= 2);
    assert.equal((await delivery.find('Payment is due within 30 days.')).length, 1);

    // Table grid after reopen: four rows across the bands, two grid columns.
    assert.equal(reopened.tables.length, 1);
    const table = reopened.tables[0];
    assert.ok(table !== undefined);
    assert.equal(table.rows, 4);
    assert.equal(table.columns, 2);
    assert.ok((await delivery.find('Scope locked')).length === 1, 'list items keep their text');
    assert.ok((await delivery.find('DRAFT watermark')).length === 0);
    const stats = await delivery.stats();
    assert.ok(stats.tables >= 1 && stats.tableRows >= 4);

    // Band and span semantics: dump the reopened document through the native
    // envelope and check the structure the fill must not have disturbed. The
    // pinned DOCX import folds the footer band into the body (the row count
    // and every cell's text survive), and flattens nested list items into
    // sibling runs; both are disclosed partial-import behaviours.
    const docxEnvelopePath = join(dir, 'reopened-docx.oword');
    await delivery.save(docxEnvelopePath, { overwrite: true });
    await delivery.dispose();
    const docxEnvelope = readEnvelope(docxEnvelopePath);
    const docxTable = tableOf(docxEnvelope);
    assert.ok(docxTable.head.length >= 1, 'the header band survives the DOCX round trip');
    assert.equal(docxTable.head.length + docxTable.body.length + ((docxTable.foot as Json[] | undefined) ?? []).length, 4, 'every row survives');
    assert.ok(
        allCells(docxTable).some((cell) => cell.colspan === 2),
        'the merged header cell keeps its colspan',
    );
    assert.ok(
        allCells(docxTable).some((cell) => cellText(docxEnvelope, cell) === 'Net 30'),
        'the footer row content stays inside the table',
    );
    const docxLists = listOf(docxEnvelope);
    assert.ok(docxLists.some((list) => list.ordered === false), 'the unordered list stays unordered');
    assert.ok(docxLists.some((list) => list.ordered === true), 'the ordered list stays ordered');
    const docxItemTexts = docxLists.flatMap((list) => (list.items as Json[]).map((item: Json) => itemText(item)));
    assert.ok(docxItemTexts.includes('Milestone A') && docxItemTexts.includes('Milestone B'), 'ordered items survive');

    // 4. The review PDF paginates: page objects match the page-tree count.
    const pdfBytes = readFileSync(pdfPath).toString('latin1');
    assert.ok(pdfBytes.startsWith('%PDF-'), 'the export is a real PDF');
    const pageObjects = (pdfBytes.match(/\/Type\s*\/Page(?![s])/g) ?? []).length;
    const counts = [...pdfBytes.matchAll(/\/Count\s+(\d+)/g)].map((match) => Number(match[1]));
    assert.ok(pageObjects >= 1, 'the review PDF holds at least one page');
    assert.ok(counts.length >= 1 && Math.max(...counts) === pageObjects, 'the page tree counts every page object');

    // 5. Reopen the native project: the session continues where it stopped and
    // the full-fidelity structure — including the nested ordered list — is intact.
    const project = new OpenWord();
    const loaded = await project.open({ path: projectPath });
    assert.equal(loaded.revision, saved.revision);
    assert.ok((await project.find('ACME')).length >= 2);
    assert.equal((await project.find('{{')).length, 0);
    const projectEnvelopePath = join(dir, 'reopened-project.oword');
    await project.save(projectEnvelopePath, { overwrite: true });
    await project.dispose();
    const projectEnvelope = readEnvelope(projectEnvelopePath);
    const nested = listOf(projectEnvelope)
        .flatMap((list) => (list.items as Json[]).map((item: Json) => ({ item, list })))
        .find(({ item }) => itemText(item) === 'Milestone A' && (item.children as Json[]).length > 0);
    assert.ok(nested !== undefined, 'the ordered list keeps its nesting in the native project');
    assert.equal(nested.list.ordered, true);
    const childList = projectEnvelope.document.blocks.find((block: { id: number }) => block.id === nested.item.children[0]).block
        .List;
    assert.ok(childList !== undefined && childList.items.some((child: Record<string, any>) => itemText(child) === 'Milestone B'));

    console.log(`offer workflow ok: docx ${docxPath}, pdf ${pageObjects} pages, project revision ${loaded.revision}`);
}

await main();
