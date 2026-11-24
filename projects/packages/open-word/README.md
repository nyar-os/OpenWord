# @wae/open-word

A session-oriented TypeScript API and the `open-word` CLI for creating, revising, inspecting and exporting Word-compatible documents.

[Product overview](../../../readme.md) · [Agent reference](../../../documentation/agents.md)

## API surface

`new OpenWord(options?)` is the entry point. Construction is synchronous and opens no file, window or socket; every operation is async and reports a revision.

- **Session:** `capabilities()`, `create({ title, template, author })`, `open({ path, intent })`, `save(target, { overwrite })`, `dispose()`
- **Read:** `stats()`, `outline()`, `find(query, { limit })`
- **Edit:** `edit(operations, { expectedRevision })`, `replace(find, replacement, { caseSensitive, wholeWord, regex, limit, expectedRevision })`, `undo({ steps })`, `redo({ steps })`, `merge(path, { mode: 'append' })`
- **Templates:** `template` on `create()` starts from an existing document, and `edit([{ type: 'template.fill', values }])` substitutes `{{key}}` placeholders.
- **Review and delivery:** `inspect({ path }, { checks })`, `export({ format, target, overwrite })`
- **CLI:** `open-word capabilities | create | open | save | inspect | find | stats | outline | edit | replace | template | undo | redo | merge | export | gui`

The CLI `open` command prints a document summary and exits; it opens no window. `open-word gui` is the only GUI entry.

An edit batch is validated as a whole before the first mutation, and each applied operation advances the revision once. `expectedRevision` rejects a stale write with `document/revision-conflict`.

## Example

```ts
import { OpenWord } from '@wae/open-word';

const word = new OpenWord();
await word.create({ title: 'Account review' });
await word.edit([{ type: 'heading.append', level: 1, text: 'Summary' }]);
await word.replace('draft', 'final', { wholeWord: true });
const { revision } = await word.export({ format: 'docx', target: 'account-review.docx' });
console.log(`exported revision ${revision}`);
await word.dispose();
```

## Related packages

- [`@wae/open-word-skills`](../open-word-skills/README.md) — agent skill documents.
- [`@wae/open-word-ui`](../open-word-ui/README.md) — OpenWord Studio.
- Platform addons `@wae/open-word-<platform>-<arch>` are installed automatically as optional dependencies.
