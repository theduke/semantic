# Semantic browser extension

The Semantic browser extension captures the active HTTP or HTTPS page as a
`semantic:base:web_bookmark` entity. The popup derives the initial title from the
page title, lets you edit it, and accepts an optional description. Before creating
anything, the background worker queries the `entities` collection by both class
and exact URL. Existing bookmarks are linked instead of updated or duplicated.

## Build

From the repository root, run:

```sh
make webextension-bundle
```

This installs locked Node dependencies, builds the local `@semantic/sdk`, checks
and tests the extension, and writes:

- `lib/js/webext/dist/chrome/`
- `lib/js/webext/dist/firefox/`
- `lib/js/webext/dist/semantic-chrome.zip`
- `lib/js/webext/dist/semantic-firefox.zip`

The archives are deterministic and contain `manifest.json` at their root.

## Manual installation

In Chrome, open `chrome://extensions`, enable Developer mode, choose **Load
unpacked**, and select `lib/js/webext/dist/chrome`. In Firefox, open
`about:debugging#/runtime/this-firefox`, choose **Load Temporary Add-on**, and
select either the Firefox ZIP or its `manifest.json`. Temporary Firefox add-ons are
removed when Firefox restarts; persistent installation requires a signed add-on.

Open the extension settings and enter the public base URL of the Semantic web
application, such as `http://127.0.0.1:8888`. A deployment below a path prefix is
also supported. On save, the browser asks for access to that server's scheme and
hostname. This permission covers all ports because Firefox does not support ports
in host permission patterns; requests still use the exact configured URL.

After updating from a version that included ports in permission patterns, reload
the extension and save its settings again to grant the corrected permission.

## Architecture and current limitations

- Chrome and Firefox use one bundled TypeScript source tree and separate Manifest
  V3 manifests. Chrome declares a service worker; Firefox declares a background
  script.
- All Semantic RPC calls happen in the background context through the repository's
  `@semantic/sdk` `SemanticClient` and `HttpTransport`.
- The configured server must expose the application at `<base URL>` and RPC at
  `<base URL>/api/v1/rpc`. Authentication setup is left to the deployment.
- Duplicate prevention is a best-effort lookup immediately before insert. Calls in
  this extension instance for the same server and URL are serialized. A database
  uniqueness constraint is still required to prevent races across devices or
  extension instances.
- Only ordinary HTTP and HTTPS tabs are supported. Browser-internal pages, local
  files, and extension pages are intentionally rejected.

See [PRIVACY.md](./PRIVACY.md) for the data handling and permission rationale.
