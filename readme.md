# OpenWord

![.docx](https://img.shields.io/badge/format-docx-blue)
![Preserve](https://img.shields.io/badge/preserve-styles%20%7C%20tables%20%7C%20headings-brightgreen)
![Template Fill](https://img.shields.io/badge/template-placeholder--fill-blue)
![License](https://img.shields.io/badge/license-MPL--2.0-blue)

[Package](projects/packages/open-word/README.md) · [Agent reference](documentation/agents.md) · [License](License.md)

## 💡 What is OpenWord?

OpenWord is a document workspace for Word `.docx` files. Create a report from scratch or from an existing document, fill `{{key}}` placeholders, find and replace text, duplicate or move sections, merge documents, inspect structure, then export editable DOCX or a PDF, Markdown, HTML, plain-text or RTF rendering.

The same document model backs the `open-word` command line, the TypeScript SDK, the bundled agent skill and the OpenWord Studio desktop window. An agent can start a document, you can review it in the studio, and a script can re-export it later without any of them disagreeing about what the document contains or which revision is current.

OpenWord is not a Word clone and not a full template engine. Placeholders are literal substitutions, not conditionals or loops. Macros are never executed, and `.doc`, `.dotx`, `.dotm` and `.odt` inputs are reported as unsupported. PDF export is a fixed projection that cannot be reopened for editing, while RTF import and export are partial and report their losses.

## 🚀 Getting started

### 1. Agent / prompt (recommended)

OpenWord ships an agent skill named `open-word` in `@wae/open-word-skills`. Install the CLI and the skill, then hand your agent a task in ordinary document language:

```text
Use the open-word skill to turn notes/account-review.md into a
review-ready account-review.docx. Preflight with
`open-word capabilities --json`, keep headings and tables native,
export DOCX plus a PDF preview, and report any losses.
```

Every task should start with the capability preflight. It reports what the installed build can actually read, edit and export:

```bash
open-word capabilities --json
```

The JSON result is a single `wae.cli-result/v0` envelope, so an agent can check `formats`, `checks` and `exportTargets` before promising an output.

### 2. Manual install

```bash
pnpm add @wae/open-word
pnpm exec open-word capabilities
pnpm exec open-word create --out account-review.oword --title "Account review"
pnpm exec open-word edit account-review.oword --heading "Summary" --level 1 --paragraph "Numbers held." --out account-review.oword --overwrite
pnpm exec open-word export account-review.oword --format docx --out account-review.docx --overwrite
```

The platform package for your machine is installed automatically as an optional dependency; there is no separate build step for users.

## ✨ Highlights

- **One document, four surfaces.** The CLI, TypeScript SDK, agent skill and studio all read and write the same revisioned document.
- **Real Word workflows.** Create, append headings, paragraphs, lists, quotes, code blocks and tables; insert, update, remove, move and duplicate blocks; edit table cells and rows and columns on a consistent grid.
- **Template fill without a template language.** `create --template` starts from any document that `open` accepts, and `template.fill` replaces `{{key}}` or `{{ key }}` with strings, numbers or booleans. Unknown placeholders stay verbatim.
- **Find and replace in place.** Search section titles, paragraphs, list items and table cells, with case-sensitive matching by default, optional whole-word matching, optional regular-expression patterns with `$1` capture expansion, and a total replacement limit.
- **Document review, not a score.** `inspect` runs structural checks for sections, tables, links, images, styles and fonts and reports the revision it observed.
- **Navigation built in.** `outline` returns every heading once, nested by level with stable `section-N` addresses; `stats` reports block, heading, table, word and reading-time counts.
- **Deliberate delivery.** Export `docx`, `pdf`, `html`, `md`, `txt` or `rtf`; PDF is marked non-editable, and RTF reports `import.rtf.*` / `export.rtf.*` loss codes.
- **Undo that stays honest.** History is session-local, capped at 50 mutations, and never written into a project file.
- **Merge with clean identities.** Append another document's blocks with fresh node ids while keeping its section structure.

## 📊 How OpenWord compares

| Need | Desktop Office automation | OOXML byte scripting | OpenWord |
| --- | --- | --- | --- |
| Works without Word or LibreOffice installed | No | Yes | Yes |
| One document state shared by CLI, SDK, agents and studio | No | No | Yes |
| Template filling | Word fields, macros or add-ins | Build it yourself | `{{key}}` substitution |
| Structural review built in | Manual inspection | No | `inspect` checks |
| PDF output | Export | Build it yourself | Export, marked non-editable |
| Macro handling | May execute | Not handled | Never executed |

OpenWord deliberately stops at placeholder substitution: no conditionals, loops, formulas or style merging are implied by `template.fill`.

## 🧪 Examples

- [Create an Account review report](projects/packages/open-word/README.md#example)
- [Fill a Record template from JSON values](projects/packages/open-word/README.md#api-surface)
- [Find and replace an old account name](projects/packages/open-word/README.md#api-surface)
- [Duplicate a section and its children](projects/packages/open-word/README.md#api-surface)
- [Merge two documents into one](documentation/agents.md#edit-operation-json)
- [Inspect structure and export DOCX, PDF, Markdown, HTML or RTF](documentation/agents.md#cli-entry-points)
- [Drive OpenWord from an agent](documentation/agents.md)
- [Open the OpenWord Studio window](projects/packages/open-word-ui/README.md)

## 📜 License

OpenWord is released under the Mozilla Public License 2.0. See [License.md](License.md).
