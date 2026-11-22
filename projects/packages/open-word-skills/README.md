# @wae/open-word-skills

Agent skill documents for OpenWord. The package ships Markdown only: no runtime code, no native addon and no document algorithms.

## Usage

Install it next to `@wae/open-word`, then ask your agent to use the `open-word` skill:

```text
Use the open-word skill to fill account-review.docx from notes/values.json and
report every unresolved placeholder.
```

The skill drives the `open-word` CLI and starts with `open-word capabilities --json`.

## Related

- [Product overview](../../../readme.md)
- [`@wae/open-word`](../open-word/README.md)
- [Agent reference](../../../documentation/agents.md)
