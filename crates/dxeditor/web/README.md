# dxeditor browser engine

This package is the browser-owned editing island used by `dxeditor`. It uses a
curated, exactly pinned Tiptap/ProseMirror stack and exposes only the
framework-neutral `ComponentDocumentV2` wire document at the Rust boundary.
The checked-in IIFE is loaded by `src/bridge/mod.rs`; Tiptap JSON is not a
persistence format. Legacy v1 callers are adapted at the Rust component API,
outside the browser session protocol.

Rebuild and verify it from the repository's Nix development shell:

```sh
npm ci
npm run check
npm test
npm run build
npm run test:e2e:chromium
```

The Playwright configuration also defines Firefox and WebKit projects. In CI or
on NixOS, use the browser bundle matching the pinned Playwright version:

```sh
PLAYWRIGHT_BROWSERS_PATH="$(nix build --no-link --print-out-paths nixpkgs#playwright-driver.browsers)" npm run test:e2e
```

Commit `package-lock.json` and `dist/editor.iife.js` whenever the source or a
dependency changes. Do not add formatting or content actions to a persistent
toolbar: those commands belong to the contextual surfaces in `src/index.ts`.
