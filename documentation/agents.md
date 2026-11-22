# OpenWord agent reference

Concise reference for agents driving the `open-word` CLI or `@wae/open-word` SDK. Documents are authoritative; commands and flags below are taken from `src/cli.ts`, types from `src/types.ts`.

## Discover capabilities first

Always start with the capability preflight:

```bash
open-word capabilities --json
```

`--json` is a global flag registered by `@wae/commander`; `emitResult` writes one machine envelope:

```json
{ "schema": "wae.cli-result/v0", "ok": true, "data": { "engine": {}, "project": "supported", "formats": {}, "checks": {}, "renderers": {}, "exportTargets": {} } }
```

A failure uses `{ "schema": "wae.cli-result/v0", "ok": false, "error": { "code": "...", "message": "..." } }`. For the current build the matrix is:

| Area | Supported | Partial | Unsupported |
| --- | --- | --- | --- |
| Import | `docx`, `docm`, `md`, `html`, `txt` | `rtf`, `pdf` | `dotx`, `dotm`, `doc`, `odt` |
| Edit | `docx`, `docm`, `md`, `html`, `txt`, `rtf` | `pdf` | `dotx`, `dotm`, `doc`, `odt` |
| Export | `docx`, `md`, `html`, `pdf`, `txt` | `docm`, `rtf` | `dotx`, `dotm`, `doc`, `odt` |
| Checks | `sections`, `tables`, `links`, `images`, `styles`, `fonts` | — | — |
| Renderers | — | — | `svg`, `png`, `pdf` |
| Export targets | `docx`, `pdf`, `html`, `md`, `txt` | `rtf` | `dotx` |

`project` is `supported`, and `engine.package` is `open-word-document`. Re-read the live matrix instead of hard-coding these values; a method or extension is never a capability by itself.

## CLI entry points

Binary: `open-word` (`dist/cli.js`). Global flags: `--json` (machine envelope) and `--no-interactive` are hidden framework globals; `--locale <id>` is registered by the locales loader.

| Command | Positionals | Key options |
| --- | --- | --- |
| `capabilities` | — | — |
| `create` | — | `--out <file>` (required), `--title`, `--template`, `--overwrite` |
| `open` | `<file>` | — |
| `save` | `<file>` | `--out <file>` (required), `--overwrite` |
| `inspect` | `<file>` | `--checks styles,fonts,images,sections,tables,links` |
| `find` | `<file> <query>` | `--limit <count>` |
| `stats` | `<file>` | — |
| `outline` | `<file>` | — |
| `edit` | `<file>` | `--paragraph`, `--heading` + `--level`, `--heading-id <section-N>` + `--level`, repeatable `--bullet`, repeatable `--numbered` + `--numbered-start`, `--quote`, `--code` + `--code-lang`, `--table <json>`, `--duplicate-block <id>`, `--title`, `--author`, `--expected-revision`, `--out`, `--overwrite` |
| `replace` | `<file>` | `--find`, `--with`, `--ignore-case`, `--whole-word`, `--out`, `--overwrite` |
| `template` | `<file>` | `--values <json>` (required), `--out <file>` (required), `--overwrite` |
| `undo` / `redo` | `<file>` | `--steps <count>` |
| `merge` | `<file> <other>` | — |
| `export` | `<file>` | `--format docx\|dotx\|pdf\|html\|md\|txt\|rtf`, `--out <file>`, `--overwrite` |
| `gui` | — | — |

Exit codes: `0` success, `1` a failed inspection (`passed: false`) or an action failure, `2` argument/validation failure. `undo`, `redo` and `merge` write back only when the input is `.oword`; other formats stay in memory because a one-shot CLI process has an empty history.

## Edit-operation JSON

`edit` takes a non-empty array. Parsing, validation and graph-dependent checks run against a staging copy before the committed document changes; each applied operation advances the revision once. Pass `expectedRevision` to reject a stale batch.

```json
{
  "expectedRevision": 4,
  "operations": [
    { "type": "heading.append", "level": 1, "text": "Summary" },
    { "type": "paragraph.append", "text": "Numbers held." },
    { "type": "block.duplicate", "id": "section-2" }
  ]
}
```

| Type | Fields |
| --- | --- |
| `paragraph.append` | `text` |
| `heading.append` | `level` 1–6, `text` |
| `heading.setLevel` | `id` (`section-N`), `level` 1–6; changes only the level, keeping the title and children |
| `heading.setText` | `id` (`section-N`), `text`; replaces only the title, keeping the level, children and position |
| `list.append` | `items: string[]`, `ordered?`, `start?` (ordered only) |
| `taskList.append` | `items: (string \| { text, checked? })[]`; exports keep `[ ]`/`[x]` |
| `quote.append` | `text` (non-empty) |
| `code.append` | `text` (non-empty), `language?` |
| `table.append` | `rows: string[][]` (at least one row and cell) |
| `table.setCaption` | `table`, `caption?`; omit or `null` clears, a present caption is trimmed and must not be blank |
| `metadata.set` | at least one of `title`, `author` |
| `document.replaceText` | `find` (non-empty), `replace`, `caseSensitive?`, `wholeWord?`, `regex?`, `limit?` |
| `block.remove` | `id` (`section-N` or `node-N`) |
| `paragraph.insert` | `index`, `text` |
| `paragraph.update` | `id` or `index`, `text` (target must be a paragraph) |
| `block.move` | `id`, `index` |
| `block.duplicate` | `id`; returns `newId` and the copy's depth-first `index` |
| `table.setCell` | `table`, `row`, `column`, `text` |
| `table.insertRow` | `table`, `row`, `cells?` (must match the grid width) |
| `table.deleteRow` | `table`, `row` (last row rejected) |
| `table.insertColumn` | `table`, `column`, `cells?` (one per row) |
| `table.deleteColumn` | `table`, `column` (last column rejected) |
| `template.fill` | `values: { [key]: string \| number \| boolean }` (non-empty) |

`block.duplicate` deep-copies the addressed block and its whole subtree with fresh node ids, then inserts the copy directly after the source in the same container (or at the top level when the source has no container). Inline content, styles and table structure are copied; asset references stay shared. `table.*` indices are zero-based: `table` counts document tables, `row` counts head, body and foot rows in order, `column` is a grid column. Inserted cells use `colspan`/`rowspan` 1, and an inserted column adds one grid column to every row.

`template.fill` replaces `{{key}}` and `{{ key }}` in titles, paragraphs, list items and table cells. Unknown placeholders stay verbatim. One fill batch advances the revision once.

`document.replaceText` matches inside one inline run, so styles and links never split. With `regex: true`, `find` is a Rust regular expression and `replace` expands `$1`/`$name` capture groups; `caseSensitive: false` compiles it with the case-insensitive flag, and `wholeWord` is rejected because boundaries belong in the pattern. `replace()` on the session accepts the same options.

## Key types and files

| Name | Where | Notes |
| --- | --- | --- |
| `OpenWord` | `src/open-word.ts` | Session class; sync constructor; async methods |
| `Document` / `DocumentSummary` | `src/types.ts` | `id`, `revision`, `title`, `sections`, `paragraphs`, `tables` |
| `DocumentCapabilities` | `src/types.ts` | `capabilities()` result |
| `EditOperation` | `src/types.ts` | Discriminated union above |
| `EditReport` | `src/types.ts` | `document`, `revision`, `results[]` |
| `FindHit` | `src/types.ts` | `blockId`, `blockIndex`, `path`, `start`, `end`, `text` |
| `ReplaceReport` | `src/types.ts` | `replaced`, `affected[]`, `document`, `revision` |
| `InspectReport` | `src/types.ts` | `passed`, `checkerVersion`, `issues[]` (no aesthetic score) |
| `StatsReport` | `src/types.ts` | Blocks, paragraphs, headings, tables, rows, words, characters, reading minutes |
| `DocumentOutline` | `src/types.ts` | Nested `OutlineItem[]` with `section-N` ids |
| `OpenWordError` | `src/errors.ts` | Stable `code`, `recoverable`, `details` |
| Commands | `src/cli.ts` | Maps each command onto one session method |
| Format registry | `src/formats.ts` | Classification only, not a support claim |

## Stable error codes

| Code | Meaning |
| --- | --- |
| `session/disposed` | The session was disposed; create a new instance |
| `session/argument-invalid` | Bad or missing argument; often also a failed undo/redo step count |
| `session/budget-exceeded` | A declared resource budget was exceeded during import |
| `document/not-found` | No document is open, or the handle is unknown |
| `document/revision-conflict` | `expectedRevision` did not match the live revision |
| `object/not-found` | The addressed block or object does not exist |
| `object/unsupported` | A known object or mode is not supported (for example a non-append merge) |
| `format/unsupported` | The format has no route in this build |
| `format/invalid` | The file could not be read, parsed or written |
| `capability/not-implemented` | No native binding is installed, so the capability cannot run |

## Repository map

- `projects/crates/model` — `open-word-document`: routing, inspection, edits, replace, outline, merge, export dispatch.
- `projects/crates/napi` — `open-word-napi`: Node-API exports and the WebView host.
- `projects/packages/open-word` — `@wae/open-word`: CLI, SDK, types, errors, format registry, GUI host.
- `projects/packages/open-word-ui` — VMZ studio source, staged into `dist/gui`.
- `projects/packages/open-word-skills/skills/open-word/SKILL.md` — agent workflow guidance.
- `projects/packages/open-word-<platform>-<arch>` — prebuilt addon packages, installed as optional dependencies.
- `projects/sites/homepage` — product homepage (VMZ).
- `documentation/README.md` — build, release and CI details.

## Guardrails

- Never infer support from a method name, command, extension or file name. Call `capabilities()` first.
- Do not add a parser, writer, XML, ZIP, style or conversion implementation to this repository; those belong to the pinned engine.
- `open()` accepts `intent: 'edit'` only. `template` and `read` intents reject with `format/unsupported`.
- `create --template` starts from any document `open` accepts and only applies optional title/author; it is not a template engine.
- PDF import is partial, and PDF export is a non-editable projection. Do not claim a PDF can be reopened for editing.
- RTF is partial in both directions. Report `import.rtf.*` and `export.rtf.*` loss codes instead of presenting the result as lossless.
- Undo/redo history is session-local, capped at 50 mutations, and never stored in `.oword`. A fresh CLI process starts with empty history.
- `merge` supports `mode: 'append'` only; it deep-copies the donor's blocks with fresh ids and keeps its section structure.
- SDK `save()` defaults to `overwrite: false`; SDK `export()` defaults to overwriting unless `overwrite: false` is passed. The CLI refuses an existing target unless `--overwrite` is given.
- Do not claim a check ran, fabricate previews, or flatten pages to hide editability problems.
