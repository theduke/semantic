# Browser testing

Rebuilt app/server against isolated `/tmp/semantic-task-browser-20260930`. Preview MCP calls returned `PreviewAutomationNoAvailableHostError`, so used the installed Playwright module and Chromium through Nix.

Commands:

```sh
nix develop .#ui -c bash -c 'SEMANTIC_DATA_DIR=/tmp/semantic-task-browser-20260930 SEMANTIC_INTERFACE=127.0.0.1 SEMANTIC_PORT=8888 cargo run --quiet --package semantic_server'
nix develop .#ui -c dx serve --web --package semantic_ui --no-default-features --features web --port 8080 --open false --watch false --hot-reload false
nix develop .#ui -c node /tmp/semantic-task-browser-evidence/flows.cjs
nix develop .#ui -c node /tmp/semantic-task-browser-evidence/acceptance.cjs
nix develop .#ui -c node /tmp/semantic-task-browser-evidence/note.cjs
nix develop .#ui -c node /tmp/semantic-task-browser-evidence/polish.cjs
```

Scripts beside this document retain the actual UI interactions; acceptance.cjs uses the created task ID in the isolated fixture. Server was restarted with `SEMANTIC_TASKS=false SEMANTIC_COMMENTS=true` against the same database for disabled.cjs.

Passed flows: fresh empty workspace; rich task create/priority/day due date and reload; empty title validation with retained draft; complete/reopen progress; explicit progress and clearing due date/reload; two subtasks and one nested child; parent/child navigation; comment root/reply/canceled reply/edit/deleted parent with intact child; flat/threaded presentation; search/status/priority/overdue/sort/clear/no-results; archive/restore; standalone Note comment create/edit/delete/reload. 1440px desktop and 390px mobile showed no horizontal overflow. Keyboard Tab reached a linked task row. Main acceptance page and console error lists were empty. Overdue filtering produced the expected single incomplete due task.

Browser findings fixed: select accessible name ambiguity, excessive readonly comment editor height, composer framing, visible composer labels. After readonly polish, rendered reply document height was 29.75px. Native/web/no-markdown checks pass.

Evidence under `/tmp/semantic-task-browser-evidence`:

- create-desktop.png (create form; fresh empty state was exercised and visually inspected before fixture creation)
- populated-workspace-desktop.png
- workspace-mobile.png
- populated-detail-desktop.png
- detail-mobile.png
- disabled-tasks.png
- result.json (actual created fixture URLs, zero browser errors)
- note-url.txt

The final tiny visible composer label/group accessibility improvement was compile-checked after the last screenshots; independent review should refresh screenshots if desired. No deployment or persistent user database was touched.

Disabled feature browser verification passed: task navigation hidden and /tasks deep link graceful despite retained schema/data; independent standalone Note comments still accepted a new comment. No page errors. The server was then restored to default enabled features.
