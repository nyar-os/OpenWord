---
name: open-word
description: Create, revise, review and deliver Word documents with OpenWord. Use when a task involves document authoring, template fill, conversion, structure review or DOCX delivery.
---

# open-word

## Status

Scaffold. `@wae/open-word` declares the session, the format registry and the
create/open/inspect/export surface, while every domain operation rejects with
`capability/not-implemented`. `open-word capabilities` reports the declared
matrix with every format, check, renderer and export target marked
`unsupported`; `open-word gui` opens the OpenWord Studio shell, which stays on an
empty state without a document engine.

Document parsing, template rendering, style resolution, conversion and DOCX
writing are unimplemented. Do not present those routes as executable, and do not
fill the gap with local XML, ZIP, style or conversion code.

## Preflight

Once the SDK/CLI ships, every task starts here:

1. Query the capabilities the installed SDK/CLI reports and confirm the format,
   object and export entries the task needs. A method name alone is not a
   capability.
2. Confirm the locale and the assets the task needs (fonts, images, data, brand
   template) are readable before generating anything.
3. If the preflight reports a missing capability, stop and report it. Never
   degrade silently or swap in an unrelated tool.

## Routes

| Route | Intent | Minimum evidence |
|-------|--------|------------------|
| create | Build a document from a brief, materials or data | per-section outline, rendered pages, per-section review |
| edit | Change an existing document without losing style or identity | revision-scoped change, re-render of affected sections, editability check |
| template | Fill a branded template through its placeholders | placeholder coverage report, overflow and style diagnostics |
| review | Audit structure, styles, tables, links and assets | diagnostics plus actual page review, not a single score |
| deliver | Export editable DOCX plus needed previews and a report | independent reopen, loss report, remaining limitations |

## Hard rules

- Never export a document whose pages are flattened images to dodge editability
  problems.
- Never claim a check ran when it did not, and never fabricate previews.
- Word or LibreOffice may serve only as an optional independent verification
  source. The workflow must not require either.
- Prefer native styles, paragraphs, tables and links over pasted pictures.
- Structural and editorial quality still require reading the rendered pages.

## Safety

- Confirm with the user before any command that writes or overwrites a document
  file, and state the target path first.
- Never delete, flatten or re-encode source assets without explicit
  confirmation, and never overwrite an input document in place unless asked.

## References

None yet. Design rules, the support matrix and executable examples ship with
the first implemented release.
