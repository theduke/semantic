# Upload — `/upload`

## Current implementation

`UploadPage` supports a multi-file queue, duplicate suppression, MIME/title inference, per-file title/description, shared destination directory/parent entity, per-file/all upload, progress, retry/remove/clear, and created entity/file links/previews (`crates/ui/src/views/upload.rs:82-599`).

## Strengths

- Rich state model and progress phases.
- Directory picker and entity autocomplete reuse existing domain queries.
- Successful results link back to semantic entities and raw files.
- Native/wasm transports stream appropriately without loading all native bytes first.

## Findings

| Priority | Finding | Evidence / impact |
|---|---|---|
| P0 | Command loop blocks queue interactions | UploadOne/All await transfers in the coroutine; metadata/add/remove commands queue behind long work (`upload.rs:93-201`). |
| P0 | Hidden partial-failure warning | Directory-link failure is stored on a Done item, immediately excluded from visible queue (`:238-248, 579-593`). |
| P1 | File input has no designed affordance | Label contains only native input; no drop copy, drag/drop, paste, types/size guidance (`:332-337`). |
| P1 | Validation/duplicates are silent or late | Duplicate files are ignored and `notice` is never populated; 100 GiB validation occurs only after Upload (`:99-127, 538-545`). |
| P1 | Form accessibility | Title/description use placeholders rather than associated labels (`:417-447`). Progress is a div without progress value semantics (`:474-507`). |
| P1 | Aggregate signal rerenders | One `Signal<Vec<UploadQueueItem>>` is filtered/cloned and rewritten per metadata keystroke/progress tick (`:87, 232-254`). |
| P1 | Missing control | No cancel/pause, bounded parallelism, failed-only retry, per-item destination override, or aggregate progress. |
| P2 | Timing ambiguity | Shared destination/parent is read when work executes, not captured at click/enqueue; queued work can receive unexpected later settings. |

## Target experience

- Designed drop zone: Choose files, drag/drop, clipboard paste, size/type guidance, duplicate/invalid summary.
- Queue grouped by Ready, Uploading, Needs attention, Complete. Each row has labelled metadata, destination override, progress/status, Upload/Cancel/Retry/Remove.
- Queue controls: Upload ready (bounded 2–4 concurrency), Cancel active when transport supports it, Retry failed, Clear completed, aggregate count/bytes/progress.
- Shared defaults are explicitly “apply to queued items” or captured per upload request; changes never silently affect already-started items.
- Completion retains compact success/warning records. Partial directory-link failure remains visible with Retry linking action.
- Drag/drop/paste and filename-conflict/duplicate policy are explicit; resumability is deferred until transport supports it.

States: empty drop zone, validating, queued, reading, uploading, finalizing, cancelling, success, partial success, failed/retry, duplicate skipped/replace decision, connection loss.

## Component boundaries and reuse

Consume `PageHeader`, `AsyncState`, `InlineNotice/Toast`, `ConfirmAction`, `EntityPicker`, `DirectoryPicker`, `FormField`, and shared `JobProgress`. Upload evolves `JobProgress { state, current, total, label }` for future Settings jobs and `DropZone { accept, multiple, disabled, on_files, on_rejected }` for import/file forms.

Suggested domain boundaries:

- `UploadController` owns keyed queue and worker permits; API `add_files`, `update_metadata`, `start(ids)`, `cancel(id)`, `retry(id)`, `remove(id)`.
- `UploadQueueRow { item: ReadSignal/Arc<UploadItemView>, on_command }` owns no full queue.
- `UploadDefaults { value, disabled, on_change, on_apply }`.
- `UploadResults { records: Rc<[UploadResultRecord]>, on_retry_link }`.

## Dioxus state architecture

- Store items in keyed `BTreeMap<QueueItemId, UploadItemState>` plus stable order, or per-row signals/controllers. High-frequency progress updates must invalidate only the row and aggregate memo.
- A small bounded worker task/semaphore starts uploads outside the command receiver. Synchronous queue commands remain responsive; each task has cancel/generation identity.
- Capture immutable `UploadRequestOptions` at Start; local draft defaults remain separate signals.
- `Memo`s derive visible group IDs and aggregate progress from lightweight state; rows read only their item.
- Effects handle drop/paste listeners and cleanup, plus optional persistence; no effect mirrors queue groups.
- Use stable queue/result keys. Do not clone `FileData` or full result objects through every render unless unavoidable.

## Visual direction

Lead with a generous but lightweight dashed drop zone, followed by a compact queue rather than many heavy cards. Status badges/progress bars use shared semantic tokens; metadata fields align consistently and completed records collapse into a quieter history. Aggregate progress remains sticky only while work is active.

## Accessibility/responsive/performance

- Drop zone is keyboard-operable and never the only file-selection method. Labels, descriptions, validation, `progressbar` values, and restrained live announcements are required.
- At narrow width each item becomes a single-column card with actions reachable; filename/ID wrap safely.
- Stress-test thousands of queued metadata records and multiple progress streams; virtualize large queues and bound concurrency/memory.

## Missing functionality

Drag/drop, clipboard, early validation, cancel, bounded parallelism, aggregate progress, duplicate/conflict policy, per-file overrides, tags/batch metadata, resumability, and persistent failed history.

## Acceptance criteria

- Long uploads never block adding/removing/editing other queued items.
- Partial success remains visible and retryable; duplicate/oversize files produce clear feedback before upload.
- Progress updates rerender only affected row/aggregate, and queue remains responsive with 1,000 items.
- Every file control/status is keyboard and screen-reader accessible.
- `JobProgress`, `DirectoryPicker`, `EntityPicker`, notices, and confirmation components are shared rather than upload-only forks.
