# @wae/open-word

TypeScript authoring surface for OpenWord: one document model behind templates,
inspection and export. The session entry point is `new OpenWord(options?)`.

## Status

The package exports the document types, the stable error codes, the format
registry (`.docx`/`.docm`/`.dotx`/`.dotm`/`.doc`/`.rtf`/`.odt`/`.txt`/`.md`/
`.html`/`.pdf`) and the authoring session `new OpenWord(options?)`.

Engine calls go through the native binding: create and open documents, inspect
structure, search text with per-block character offsets, apply atomic edits
(append, insert, replace, remove, paragraph updates, block moves, table cell and
row/column changes), undo/redo those edits, merge another document into the open
one, save project containers and export `docx`/`md`/`html`/`pdf`/`txt`/`rtf`.

Undo and redo are session state: each successful edit (and each merge) records
the prior graph, the history is capped at 50 mutations, a new edit discards the
redo path, and `document.undo`/`document.redo` bump the revision once. History
is never written to a `.oword` container, so closing the document releases it.
The CLI commands `undo <file> [--steps]` and `redo <file> [--steps]` operate on
the opened session. Because a one-shot CLI process opens a fresh session, it has
an empty history and the command reports that instead of inventing a mutation
log; when a session reuses a handle, only a `.oword` input is written back to its
own file (other formats are reported as in-memory only).

`document.merge` opens the donor through the same routing as `open()`
(`docx`/`docm`/`md`/`html`/`rtf`/`txt`/`.oword`), deep-copies its blocks to the
end of the receiver with fresh node ids, keeps the donor's section structure,
bumps the revision once and returns `{ blocksAdded, summary, revision, losses }`.
Only `append` is supported; anything else rejects with `object/unsupported`.

`.rtf` opens through the built-in reader as a partial text projection: paragraphs,
line breaks, common character styles, cp1252 escapes and `\uN` code points are
kept; tables, lists, pictures, fields, colors and the font table are reported as
import losses. RTF export writes those same constructs: tables degrade to
tab-separated paragraph rows (`export.rtf.table`), pictures are skipped
(`export.rtf.image`) and link destinations are dropped while their text stays
(`export.rtf.link`). All import and export codes are listed in
`formats.rtf.lossCodes`, and `formats.rtf.export` is `partial`.

Without a built binding every engine call rejects with
`capability/not-implemented`. Classification is not a support claim: reads,
writes and export targets come from `capabilities()`, never from an extension or
a method name.

## Boundaries

- No DOCX parsing or writing, styles, templates, rendering, ZIP or XML code in
  this package.
- No second document model: the CLI, the studio and the agent skills consume the
  same document revision.
- Native access goes through the platform binding package.

## Scripts

```bash
pnpm --filter @wae/open-word run typecheck
pnpm --filter @wae/open-word run build
pnpm --filter @wae/open-word run test
```
