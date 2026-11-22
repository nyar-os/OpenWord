# @wae/open-word-linux-x64

Prebuilt Linux x64 native addon for [`@wae/open-word`](../open-word/README.md). **Not a public import target.** It is installed automatically as an optional dependency on matching `os` and `cpu`; application code imports `@wae/open-word`.

## Example

```ts
import { OpenWord } from '@wae/open-word';

const word = new OpenWord();
await word.create({ title: 'Account review' });
```
