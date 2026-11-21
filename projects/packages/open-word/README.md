# @wae/open-word

TypeScript authoring surface for OpenWord: one document model behind templates,
inspection and export. The session entry point is `new OpenWord(options?)`.

## Status

The package exports the document types, the stable error codes, the format
registry (`.docx`/`.docm`/`.dotx`/`.dotm`/`.doc`/`.rtf`/`.odt`/`.txt`/`.md`/
`.html`/`.pdf`) and the authoring session `new OpenWord(options?)`.

Engine calls go through the native binding: create and open documents, inspect
structure, apply atomic edits, save project containers and export
`docx`/`md`/`html`/`pdf`. Without a built binding every engine call rejects with
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
