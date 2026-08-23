# UI Feature Gap Analysis

This analysis compares the primary old SolidJS UI in `../semantic-2/js/ui` with the current Dioxus UI in `crates/ui` and `crates/ui_core`.

The new UI already covers the core foundation: catalog browsing, collections, entity detail/create/edit/delete, SQL querying, card/table browsing, directory/tree operations, uploads with progress, and extensible schema-driven rendering and forms.

## Functionality missing from the new UI

### 1. Structured browse filters

The old browser provided a structured filter builder with:

- Free-text entity search.
- Multi-select filtering by entity class/type.
- Multi-select filtering by tags.
- Sorting by any attribute, ascending or descending.
- Structured and raw-SQL filter modes.
- Debounced automatic refresh.
- Filter state encoded in the URL.
- “Load more” pagination.

Relevant old implementation:

- `../semantic-2/js/ui/src/component/entity/filter/EntityFilterForm.tsx`
- `../semantic-2/js/ui/src/component/entity/filter/SortSelector.tsx`
- `../semantic-2/js/ui/src/component/entity/BrowsePage.tsx`

The new browser has raw SQL editing, collection selection through its route, card/table modes, page size, grid width, and previous/next pagination. It does not provide a user-friendly query builder. The implementation is in `crates/ui/src/views/browse.rs`.

Missing:

- Search box.
- Type/class filters.
- Tag filters.
- Attribute sort selector.
- Structured filter-to-SQL generation.
- Readable/shareable filter parameters. The current custom SQL URL only contains a local-storage hash, so it is not portable between browsers.

Prerequisite: mostly UI and query-generation work. The current SQL query interface is sufficient.

### 2. Global entity search

The old navbar offered a keyboard-enabled entity search modal. Search accepted titles and entity UUIDs, displayed matching entities, and allowed direct navigation.

Relevant old implementation:

- `../semantic-2/js/ui/src/component/entity/EntitySearcherModalToggle.tsx`
- `../semantic-2/js/ui/src/component/entity/search.tsx`

The new shell in `crates/ui/src/components/shell.rs` has no global search or command palette.

Prerequisite: none beyond the existing database query RPC.

### 3. Tagging workflow

The old UI had two distinct tag features:

- Per-entity tag editing through a searchable multi-select modal.
- A tag administration page supporting creation, deletion, and merging.

Relevant old implementation:

- `../semantic-2/js/ui/src/component/tag/EntityTagManager.tsx`
- `../semantic-2/js/ui/src/component/tag/TagManager.tsx`

The new UI has generic entity CRUD, so a tag entity can theoretically be edited as an ordinary record, but it lacks the purpose-built workflows:

- Add/remove tags from an entity.
- Search available tags.
- Create a tag while tagging.
- Manage all tags.
- Merge one tag into another and rewrite references.
- Filter browse results by tags.

Prerequisites:

- Tag class/attribute definitions in the active catalog.
- Batch update support for assigning/removing references is already available.
- Tag merge needs a higher-level operation or carefully implemented transactional batch logic.

### 4. Media player and slideshow

The old `/play` page provided:

- A media queue assembled from filters.
- Audio, video, and image playback.
- Play/pause, previous/next, mute, cycling, shuffle, and timed advancement.
- Append or replace queue contents.
- Full-screen mode.
- Keyboard controls.
- Current-item progress/duration.
- Opening and editing the current entity.
- Expanding container entities to their related media.

Relevant old implementation:

- `../semantic-2/js/ui/src/component/play/PlayPage.tsx`
- `../semantic-2/js/ui/src/component/play/Player.tsx`
- `../semantic-2/js/ui/src/component/play/PlayerBar.tsx`

The new rendering catalog contains media abstractions and media handles, but there is no end-user playback route or queue UI.

Prerequisites:

- Complete audio/video media renderer registrations in the new catalog.
- Relation traversal/query conventions for “expand to media.”
- Player state and queue abstractions.

### 5. URL importer

The old `/import` workflow could:

- Fetch metadata from a URL and preview candidate entities.
- Detect entities already imported from the same URL.
- Hide existing entities.
- Import or discard candidates individually.
- Optionally download associated media.
- Follow “load more” and related URLs.
- Display successfully imported entities.

The old implementation is in `../semantic-2/js/ui/src/component/imports/Importer.tsx`.

There is no importer route or related UI in the new implementation.

This is primarily blocked by higher-level functionality. The new app only registers scope, database, batch, and single-file analysis commands; it has no fetch/import command family. Command registration is in `crates/app/src/command.rs`.

Prerequisites:

- URL extractor/provider interface.
- Fetch-preview result types.
- Import command and media-import policy.
- Existing-source lookup convention.
- Optional asynchronous job support for slow imports.

### 6. Blob-store cleanup

The old settings UI could:

- Find unreferenced blobs.
- Show their paths, sizes, and total reclaimable space.
- Delete all unused blobs.
- Refresh the result afterward.

The old implementation is in `../semantic-2/js/ui/src/component/settings/blob_cleanup.tsx`.

The new app supports file creation/read and object stores, but exposes no blob enumeration, reference audit, or deletion RPC.

Prerequisites:

- Object-store listing and deletion APIs.
- A reliable database-to-blob reference scanner.
- Dry-run result type containing key and size.
- Cleanup RPC with explicit confirmation and safety semantics.

### 7. Bulk media analysis

The old settings page could start analysis of all media, then poll and display queued/running/progress/success/failure state. The implementation is in `../semantic-2/js/ui/src/component/settings/media_analyzer.tsx`.

The new backend supports analyzing one persisted file through `semantic.file.analyze`, but there is no bulk-analysis command, job system, or UI. The relevant new implementation is in:

- `crates/app/src/command.rs`
- `crates/app/src/media.rs`

Prerequisites:

- Enumerate analyzable files.
- Bulk job orchestration.
- Progress/event/status types.
- Retry/cancellation policy.
- UI for starting and monitoring jobs.

A simpler initial UI could run single-file analysis from an entity action without adding jobs.

### 8. Similar/duplicate image finder

The old UI allowed configuring:

- Minimum similarity.
- Maximum similarity.
- Maximum results.
- Background execution with streamed matches.
- Side-by-side image match presentation.

The old implementation is in `../semantic-2/js/ui/src/component/settings/similar_image_finder.tsx`.

The new system has media metadata analysis but no perceptual-image index, similarity service, job events, or UI.

Prerequisites:

- Image fingerprint/embedding type.
- Persistent or rebuildable similarity index.
- Similarity query service.
- Background job and event-stream API.
- Match result types.

### 9. Upload metadata and clipboard ingestion

The new uploader is substantially better in several respects: it has per-file title/description editing, upload progress, retryable errors, upload-all, deduplication, MIME inference, and result links. It is implemented in `crates/ui/src/views/upload.rs`.

Old functionality still missing:

- Add files by clipboard paste.
- A global metadata form applied to the batch.
- Choose a semantic collection entity for uploaded files.
- Choose a parent entity/directory before upload.
- Potentially assign tags during upload.

The old implementation is in `../semantic-2/js/ui/src/component/upload/Uploader.tsx`.

Prerequisites:

- Clipboard support is UI-only.
- Collection routing is mostly available.
- Parent/tag metadata requires an agreed mapping between upload metadata and the current file/directory/relation model.

### 10. Authentication/login and scope selection

The old UI gated the application behind an initial server/schema probe and cached the schema locally. It also exposed a logout-looking navbar action, although that action appears to route to the tag manager rather than actually clearing a session.

Relevant old implementation:

- `../semantic-2/js/ui/src/App.tsx`
- `../semantic-2/js/ui/src/component/LoginPage.tsx`

The new UI expects an RPC client and optional scope to be supplied during launch. The backend supports listing, opening, selecting, and inspecting scopes, but the UI has no:

- Connection/login screen.
- Connection failure recovery.
- Scope/database chooser.
- Current-scope indicator.
- Scope switching.
- Logout/disconnect action.

Prerequisite: decide whether the product needs authentication, remote server selection, local database selection, or only scope selection. Scope RPC support already exists.

### 11. Extra/dynamic attributes while editing

The old generic entity form could add attributes beyond the selected class’s declared fields using an attribute search control. The implementation is in `../semantic-2/js/ui/src/component/entity/entity_form.tsx`.

The new class form is strongly catalog-driven. Existing undeclared fields can be rendered generically, but there is no obvious UI for searching the catalog and adding an arbitrary attribute during creation/editing.

Prerequisite: a policy decision. This should only be restored if open-schema entities are intentional; otherwise the stricter new behavior is preferable.

### 12. Settings/tool hub and job UI

The old settings route served as an entry point for tags, blob cleanup, media analysis, and duplicate detection. It also had reusable job status/error components.

Relevant old implementation:

- `../semantic-2/js/ui/src/component/SettingsPage.tsx`
- `../semantic-2/js/ui/src/component/job/JobStatusLoader.tsx`

The new UI has no settings/maintenance route or common long-running-task UI.

Prerequisite: introduce this when the first maintenance command is implemented.

## Things to add

The following order balances user value, implementation cost, and prerequisite dependencies.

### 1. Global entity search

- Search dialog in the shell.
- Keyboard shortcut.
- Title/ID matching and direct navigation.

This has low backend cost and high everyday value.

### 2. Structured browse filtering

- Search, class filter, sort, and later tag filter.
- Compile controls to SQL.
- Put either the actual filter model or SQL in shareable URL state.
- Preserve the existing raw-SQL mode as an advanced tab.

### 3. Scope/database chooser

- Current scope indicator.
- List/open/switch UI.
- Friendly initial connection and catalog-load error state.

### 4. Entity tagging

- Register “Tags” as an entity action.
- Searchable add/remove dialog.
- Add tag filtering to Browse.
- Then add tag creation and administration.
- Implement merge only after transactional semantics are defined.

### 5. Upload enhancements

- Clipboard paste.
- Batch defaults for destination collection/directory.
- Optional tags.
- Reuse the existing directory entity picker/query machinery.

### 6. Single-file media analysis action

- Add “Analyze media” to file entity detail/card actions.
- Show returned metadata and refresh the entity.
- Use the existing `semantic.file.analyze` command and avoid job infrastructure initially.

### 7. Reusable background-job subsystem

- Job ID and queued/running/finished/failed/cancelled states.
- Progress percentage/message.
- Status polling or event subscription.
- Cancellation and retention rules.
- Shared Dioxus job-status component.

### 8. Bulk media analysis

- Build on the job subsystem.
- Include selection/filter options rather than only “all media.”

### 9. Blob audit and cleanup

- Start with a read-only report.
- Show blob key, size, reference status, and totals.
- Add destructive cleanup only with confirmation and revalidation.

### 10. URL import provider architecture and importer UI

- Preview first, then explicit per-item import.
- Preserve existing detection, related URLs, media toggle, and imported-results behavior.
- Keep provider-specific extraction outside the core UI.

### 11. Media player

- First implement queue plus image/audio/video controls.
- Then filtering, relation expansion, fullscreen, shortcuts, shuffle, cycling, and timed slides.

### 12. Similar-image detection

- Implement after indexing, jobs, event/results types, and media-domain support are available.

## Suggested parity milestone

The most valuable first parity milestone is items 1–6 above:

1. Global entity search.
2. Structured browse filtering.
3. Scope/database chooser.
4. Entity tagging.
5. Upload enhancements.
6. Single-file media analysis.

Items 7–12 are not merely missing screens; they require deliberate additions to the application service and RPC layers.
