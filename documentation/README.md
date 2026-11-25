# OpenWord maintainer guide

Maintainer-only material: layout, build, release and CI. User-facing documentation lives in the [root README](../readme.md), the [`@wae/open-word` package README](../projects/packages/open-word/README.md) and the [agent reference](agents.md).

## Project layout

```text
OpenWord/
├── projects/
│   ├── crates/
│   │   ├── model/                    # crate open-word-document
│   │   └── napi/                     # crate open-word-napi (cdylib)
│   ├── packages/
│   │   ├── open-word/                # @wae/open-word: TypeScript session + CLI + GUI host
│   │   ├── open-word-ui/             # @wae/open-word-ui: VMZ studio, staged to dist/gui
│   │   ├── open-word-skills/         # @wae/open-word-skills: skill documents only
│   │   ├── open-word-win32-x64/      # prebuilt native addon packages
│   │   ├── open-word-darwin-x64/
│   │   ├── open-word-darwin-arm64/
│   │   ├── open-word-linux-x64/
│   │   └── open-word-linux-arm64/
│   ├── sites/
│   │   └── homepage/                 # @wae/open-word-homepage (VMZ)
│   └── sub_projects/                 # Cargo workspace member + cargo publish helper
├── wae.config.ts                     # native targets, build order, staging, skills, CI binding
├── pnpm-workspace.yaml
├── Cargo.toml
└── package.json
```

- **`projects/crates/model` (`open-word-document`)** owns the product surface: the live handle store, format routing, summary projection, rule inspection, atomic edits, replace, outline, merge and export dispatch. It binds the pinned upstream document engine and never re-implements a format adapter.
- **`projects/crates/napi` (`open-word-napi`)** is a thin Node-API `cdylib`. Every export returns a JSON string (document calls) or a boolean (`document_close`), and it also hosts the WebView window (`openGui`, `pumpEvents`, `closeGui`, `isGuiOpen`). Documents opened from the CLI and the studio share one handle table and one revision.
- **`projects/packages/open-word`** is the published surface: `src/open-word.ts` (session), `src/cli.ts` (command assembly), `src/service.ts` (native calls), `src/types.ts` (authoring types), `src/errors.ts` (stable codes), `src/formats.ts` (format registry), `src/native.ts` (addon discovery), `src/gui.ts` (window host). `locales/en-US/cli.json` holds CLI text.
- **`projects/packages/open-word-ui`** builds `dist/static` with `vmz build . --profile static`; `wae.config.ts` stages it to `projects/packages/open-word/dist/gui`.
- **`projects/packages/open-word-skills`** ships `skills/open-word/SKILL.md` only. It has no runtime code and no conversion algorithms.
- **Platform packages** are `@wae/open-word-<platform>-<arch>`; each holds one `.node` file plus `README.md` and `License.md`, with `os`/`cpu` fields set in `package.json`.
- **`projects/sites/homepage`** is the VMZ product site.
- **`projects/sub_projects`** is a Cargo workspace member whose `package.json` exposes `cargo publish --allow-dirty` as `pnpm p`.

## Build and test

Prerequisites: Node `>=18`, pnpm `10.28.0` (declared in the root `package.json`) and Rust `1.90` (declared in `Cargo.toml`, with `rust-toolchain.toml` in the repository).

```bash
pnpm install                 # link the workspace and install dependencies

wae native build             # build the native addon into the platform packages
wae native stage             # stage the built addon into its package
wae build                    # native -> ts -> ui -> skills -> staging (see wae.config.ts)
pnpm build:napi              # alias for `wae native build`
pnpm build:ts                # build @wae/open-word with tsc
pnpm build:ui                # build the VMZ studio

pnpm typecheck               # tsc --noEmit in packages that define a typecheck script
pnpm test                    # tsc + node --test in packages that define a test script
cargo test -p open-word-document
cargo test -p open-word-napi
pnpm check:rust              # fmt --check + clippy -D warnings + tests for both crates
pnpm fmt                     # cargo fmt --all
pnpm gui                     # build everything, then run `node ./dist/cli.js gui`
```

Build order is fixed by `wae.config.ts`: native, then TypeScript, then UI, then skills, then staging. The declared build outputs are `projects/packages/open-word/dist/cli.js` and `projects/packages/open-word/dist/gui/index.html`.

The package test script compiles `src` and `tests` with `tsc -p tsconfig.test.json` and runs `dist-test/tests/smoke.test.js`. Without a built addon, engine calls reject with `capability/not-implemented`; the smoke suite skips document-behaviour tests in that state. Document behaviour is covered by the Rust tests in `projects/crates/model/tests/` (editing, history, inspect, merge, outline, replace, round-trip and RTF).

## Studio build

`@wae/open-word-ui` is a private VMZ package. Its build is part of `wae build` (`native → ts → ui → skills → staging`); run it directly with:

```bash
pnpm --filter @wae/open-word-ui run check   # vmz check
pnpm --filter @wae/open-word-ui run build   # vmz build --profile static
pnpm --filter @wae/open-word-ui run serve   # local preview
```

`vmz build` emits `dist/static`, which the staging step copies to `projects/packages/open-word/dist/gui` for the native WebView host.

## Release and publishing

The publishable set is `@wae/open-word` plus the five `@wae/open-word-<platform>-<arch>` packages; `@wae/open-word-ui` and `@wae/open-word-skills` remain `private: true`. The app and platform packages are at version `0.0.0`, the studio is at `0.0.1`, and `@wae/open-word` references the platform packages through `workspace:*` optional dependencies.

Current state of the repository:

- `wae.config.ts` leaves `release` unset; `nifty.config.ts` declares the npm Trusted Publisher target (`repo: nyar-os/OpenWord`, `file: publish-npm.yml`, `environment: NPM_PUBLISH`).
- `.github/workflows/publish-npm.yml` builds the native addon for all five targets, stages each `.node` into its platform package, builds the TypeScript surface and GUI, and publishes through `nifty publish` on `v*` tags.
- `@doki-land/nifty` is a root dev dependency, so `pnpm exec nifty` is available locally.
- Local publish credentials live in `.secrets/nifty.json`; the whole `.secrets/` directory is git-ignored and must never be committed.

Local placeholder publishing uses `nifty publish --placeholder`; real versions come from the tagged CI workflow.

Before publishing, confirm that the platform packages were staged from a successful `wae native build`, that versions were bumped together, and that `workspace:*` dependencies are rewritten to concrete versions by the publishing tool. Do not publish from a dirty native build.

## Dependency and version pinning

- The Cargo workspace declares `version = "0.0.0"`, `edition = "2024"`, `rust-version = "1.90"` and `license = "MPL-2.0"` in the root `Cargo.toml`.
- `wae` is pinned to the full 40-character revision `f92f818fde7174154d77a87b55a5e44b367034c9`.
- The shared boost engine crates (`document-ir`, `document-session`, `boost-fs`, `document-formats`) are all pinned to the same full revision, `1c2dbba30d576794bbbe51fdd2e9f5f4f8fe59d1`, so local builds and CI resolve the same semantic IR and adapters. Move these pins together; never use a branch or floating ref.
- `document-formats` enables the `docx`, `pdf`, `html`, `markdown` and `rtf` features.
- The native build is `open-word-napi` / `open_word_napi`; `wae.config.ts` maps five targets (win32-x64, darwin-x64, darwin-arm64, linux-x64, linux-arm64) to their package directories and `.node` file names.
- Node tooling: pnpm `10.28.0`, `@wae/commander` `0.0.7`, VMZ `0.3.17`, TypeScript `^5.9.2`.
- `Cargo.lock` is present locally but listed in `.gitignore`; do not assume it is part of the published repository state.

## CI workflows

- **`check-rust.yml`** runs on push and pull request to `master` and `dev`. It builds a matrix of `ubuntu-latest`, `macos-latest` and `windows-latest`, installs the nightly toolchain with `rustfmt` and `clippy`, then runs `cargo build --workspace --release` and `cargo test --workspace --release`.
- **`check-typescript.yml`** runs on the same branches on `ubuntu-latest`: `pnpm/action-setup`, Node 22 with pnpm cache, `pnpm install --no-frozen-lockfile`, `pnpm typecheck`, `pnpm test`.
- **`rust.yml`** is an older, narrower workflow that runs `cargo build --release` and `cargo test --release` on the same three-OS matrix. Treat `check-rust.yml` as the current Rust gate if the two conflict.
- `wae.config.ts` binds CI to `nyar-os/OpenWord`, lists `check-rust.yml` and `check-typescript.yml`, requires the jobs `build` and `check`, and sets `shaBinding: false`.
