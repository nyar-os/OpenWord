/**
 * End-to-end offer workflow, the runnable form of `documentation/agents.md`.
 *
 * The script generates its own DOCX template fixture, fills it through the
 * SDK, delivers DOCX, a review PDF and the native project, then reopens every
 * artifact and asserts the delivered content: placeholders are gone, the table
 * keeps its grid, list items survive the round trip and the PDF really
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

async function main(): Promise<void> {
    if (nativeBinding() === undefined) {
        throw new Error('this example needs a built native binding; install the platform package or set OPEN_WORD_NATIVE');
    }
    const dir = mkdtempSync(join(tmpdir(), 'open-word-offer-'));
    const templatePath = join(dir, 'offer-template.docx');
    const docxPath = join(dir, 'offer.docx');
    const pdfPath = join(dir, 'offer-review.pdf');
    const projectPath = join(dir, 'offer.oword');

    // 1. Generate the template fixture: placeholders, a list and a table.
    const author = new OpenWord();
    await author.create({ title: 'Offer' });
    await author.edit([
        { type: 'heading.append', level: 1, text: 'Offer for {{customer}}' },
        { type: 'paragraph.append', text: 'Total {{amount}} due {{due}}. DRAFT terms apply.' },
        { type: 'list.append', items: ['Scope locked', 'DRAFT watermark'] },
        { type: 'table.append', rows: [['Field', 'Value'], ['Customer', '{{customer}}'], ['Amount', '{{amount}}']] },
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

    // Table semantics survive the round trip: one 3x2 grid with the filled cell.
    assert.equal(reopened.tables.length, 1);
    const table = reopened.tables[0];
    assert.ok(table !== undefined);
    assert.equal(table.rows, 3);
    assert.equal(table.columns, 2);
    assert.ok((await delivery.find('Scope locked')).length === 1, 'list items keep their text');
    assert.ok((await delivery.find('DRAFT watermark')).length === 0);
    const stats = await delivery.stats();
    assert.ok(stats.tables >= 1 && stats.tableRows >= 3);
    await delivery.dispose();

    // 4. The review PDF paginates: page objects match the page-tree count.
    const pdfBytes = readFileSync(pdfPath).toString('latin1');
    assert.ok(pdfBytes.startsWith('%PDF-'), 'the export is a real PDF');
    const pageObjects = (pdfBytes.match(/\/Type\s*\/Page(?![s])/g) ?? []).length;
    const counts = [...pdfBytes.matchAll(/\/Count\s+(\d+)/g)].map((match) => Number(match[1]));
    assert.ok(pageObjects >= 1, 'the review PDF holds at least one page');
    assert.ok(counts.length >= 1 && Math.max(...counts) === pageObjects, 'the page tree counts every page object');

    // 5. Reopen the native project: the session continues where it stopped.
    const project = new OpenWord();
    const loaded = await project.open({ path: projectPath });
    assert.equal(loaded.revision, saved.revision);
    assert.ok((await project.find('ACME')).length >= 2);
    assert.equal((await project.find('{{')).length, 0);
    await project.dispose();

    console.log(`offer workflow ok: docx ${docxPath}, pdf ${pageObjects} pages, project revision ${loaded.revision}`);
}

await main();
