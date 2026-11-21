# open-word-ui

VMZ static UI for the OpenWord Studio window. `vmz build . --profile static`
emits `dist/static`, which the repository staging step copies to
`projects/packages/open-word/dist/gui` for the native WebView host.

```bash
pnpm --filter @wae/open-word-ui run check   # vmz check
pnpm --filter @wae/open-word-ui run build   # vmz build --profile static
pnpm --filter @wae/open-word-ui run serve   # local preview
```

The page talks to the host over `chrome.webview.postMessage` JSON-RPC
(`ready`, `capabilities`, `document.create`, `document.open`, `document.save`,
`document.inspect`, `document.export`, `document.close`, `close`). Without a
WebView host it shows the empty state and "Preview mode — native window
unavailable"; it never fabricates a document.
