# @wae/open-word-ui

Desktop shell for OpenWord Studio. This private package contains the VMZ static UI that is staged into the `@wae/open-word` GUI asset; it is not published on its own.

## Usage

```bash
open-word gui
```

The window talks to the native host over `chrome.webview.postMessage` JSON-RPC (`ready`, `capabilities`, `document.create`, `document.open`, `document.save`, `document.inspect`, `document.export`, `document.close`, `close`). Without a WebView host it shows an empty state and a preview notice; it never fabricates a document.
