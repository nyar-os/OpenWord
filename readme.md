# OpenWord

![.docx](https://img.shields.io/badge/format-docx-blue)
![Preserve](https://img.shields.io/badge/preserve-styles%20%7C%20tables%20%7C%20headers-brightgreen)
![Template Fill](https://img.shields.io/badge/template-placeholder--fill-blue)

CLI/SDK for Word `.docx` authoring, template documents, and high-fidelity conversion. One document model serves the command line, the agent skills and the desktop studio, so editing, inspection and delivery never disagree about what a document contains.

## Architecture

```mermaid
graph LR
    A[.docx file] --> B[OpenWord]
    C[CLI · SDK · Agent] --> B
    B --> D[Native bridge]
    D --> E[Document processing engine]
    F[UI (VMZ)] --> B
```

## What it does NOT do

- **No `.docx` parser or writer implementation** — OOXML parsing, the style model, image handling and section math belong to the shared processing engine. This repository only binds it.
- **No document algorithms** — style inheritance, layout, template rendering, conversion and review rules live upstream.
- **No fixture corpus** — golden documents, conformance fixtures and round-trip acceptance suites live downstream. This repository keeps minimal smoke-level tests only.
- **No second document model** — the CLI, the studio and the agent skills consume the same document revision.
- **No algorithms in the skills package** — it ships skill documents only.
- **No macro, template, ODT or legacy Word input** — those formats are reported as unsupported. RTF is read by a small built-in reader as a declared text projection, never as a full word processor.

## Usage

```bash
open-word capabilities
open-word create --out report.oword --title "Research review"
open-word open report.docx
open-word save report.docx --out report.oword
open-word inspect report.docx --checks styles,tables,links
open-word find report.oword "revenue" --limit 5
open-word replace report.oword --find "old name" --with "new name" --out report.oword --overwrite
open-word edit report.oword --heading "Results" --level 2 --paragraph "Numbers held." --out report.oword --overwrite
open-word export report.oword --format pdf --out report.pdf --overwrite
open-word gui
```

```ts
import { OpenWord } from '@wae/open-word';

const word = new OpenWord();
await word.create({ title: 'Research review' });
await word.edit([{ type: 'heading.append', level: 1, text: 'Results' }]);
await word.save('report.oword');
await word.export({ format: 'docx', target: 'report.docx' });
await word.dispose();
```

`capabilities` reports what the installed engine actually routes: `.docx`/`.docm`
import and `docx`/`md`/`html`/`pdf`/`txt` export are supported, `.md`/`.html`/`.txt`/`.pdf`
open for reading (PDF import is partial), `.rtf` opens as a partial text
projection, and `.dotx`/`.dotm`/`.doc`/`.odt` are unsupported. RTF import keeps
paragraphs, `\par`/`\line` breaks, `\b`/`\i`/`\ul`/`\strike`/`\super`/`\sub`
runs, `\plain` resets, cp1252 `\'hh` escapes and `\uN` code points. Tables and
lists degrade to paragraphs, and the font table, color table, pictures and field
instructions are skipped; each loss is reported as `import.rtf.table|list|image|field|color|font`
and declared in `formats.rtf.lossCodes`.

Editing works on the shared document model: append paragraphs, headings and
tables, insert a paragraph at any depth-first position, replace text across
titles, paragraphs, list items and table cells, remove a block with its whole
subtree, update one paragraph's text by block id or depth-first index, move a
block (with its subtree) to another depth-first position, set one table cell,
and insert or delete table rows and columns. Row and column edits keep every row
on the same grid: inserted cells use `colspan`/`rowspan` 1, an inserted column
adds one grid column to every row, and deleting the last row or column is
rejected. Every operation advances exactly one revision, a batch is validated
before the first mutation, and `expectedRevision` rejects stale writes. `find`
searches the same text in document order and reports character offsets per
block. `inspect` runs structural rules (sections, tables, links, images, styles)
and reports the revision it observed. PDF export is a projection and cannot be
reopened for editing. Plain-text export uses `#` heading markers, `-` list items
and tabbed table rows.

## Repo layout

```
OpenWord/
├── projects/
│   ├── packages/
│   │   ├── open-word/              # CLI + SDK entry point
│   │   ├── open-word-skills/       # Agent skill documents
│   │   ├── open-word-ui/           # VMZ desktop studio UI
│   │   ├── open-word-win32-x64/    # Windows x64 native addon
│   │   ├── open-word-darwin-x64/   # macOS x64 native addon
│   │   ├── open-word-darwin-arm64/ # macOS ARM64 native addon
│   │   ├── open-word-linux-x64/    # Linux x64 native addon
│   │   └── open-word-linux-arm64/  # Linux ARM64 native addon
│   └── sites/
│       └── homepage/               # Product homepage
├── package.json
└── pnpm-workspace.yaml
```

This repository ships the first usable slice: project containers, Word,
Markdown/HTML/text import, partial RTF import, structural inspection, atomic
edits (including table grids and block moves), text search and
`docx`/`md`/`html`/`pdf`/`txt` export. Template rendering, macro handling and the
remaining format family are reported as unsupported until they are wired.
