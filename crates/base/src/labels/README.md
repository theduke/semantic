# Labels

Labels live in the `entities` collection. They use `semantic:base:label:name`, an optional
`semantic:base:label:color` (`#RRGGBB`), and the existing description, parent, created-at,
and updated-at attributes. Migration `006_labels` installs the schema and the indexed
external `semantic:base:entity_label` relation. Existing migrations are unchanged.

Labels (`semantic:base:label`) are assignable; label groups
(`semantic:base:label_group`) organize them and cannot be assigned. Both share the
`Label` Rust model, distinguished by `LabelKind`, and support optional description,
color, and parent. Use `Label::new` or `Label::new_group` to construct them.

A group's `selection_mode` controls its **direct child labels**: `multiple` (the
default) or `exclusive`. Top-level labels are independent. Selecting children does
not implicitly select ancestors. Ordinary labels can also have children and remain
assignable; exclusive selection is only available on groups. Names need not be
globally unique; UIs show paths.

`LabelCatalog` holds shared hierarchy and selection rules. `LabelStore` is the small
storage adapter used by `list_labels`, `labels_for_entity`, `add_labels`, `remove_labels`,
`replace_labels`, `save_label`, and `delete_label`. The app adapter resolves database
scopes using the normal authorization path. Labels can be assigned to entities in any
collection; each relation records the source collection as well as the entity ID.

Use `labels_for_entity` and the label RPC commands for collection-aware membership
lookup. The generic indexed relation predicates (`has_relation` / `has_relation_path`)
currently index bare endpoint IDs: `entities/x` and `other/x` are indistinguishable
there. They cannot provide collection-aware label lookup. Endpoint collection
authority is tracked in the graph query plan; this does not affect the label helpers,
which match both source collection and ID.

- Adding one child of an exclusive group replaces its selected sibling.
- Adding or replacing with conflicting siblings in one request is rejected.
- Adds/removes are idempotent; replacement with an empty list clears the selection.
- Missing labels/entities and invalid parents/colors are rejected before writing.
- Edits reject cycles and hierarchy/mode changes that would invalidate existing assignments.
- Deleting a leaf removes its assignments in the same batch. Move/delete children first.
- Saving retains creation time and unrelated attributes and refreshes modification time.
- Label-to-group conversion is rejected while the label has assignments. Remove those
  assignments explicitly first; converting a group to an ordinary label clears its
  child selection mode.

Migration `007_label_groups` preserves the definition/history of `006_labels` and
converts existing exclusive parent labels to groups. Existing memberships are retained,
including historical direct assignments to those parents. Such assignments remain
visible as removable legacy group chips in the editor; remove them before saving a
new selection. No new group assignment is accepted. Existing ordinary label hierarchy
and its memberships are retained.

Writes are serialized by a process-wide async mutex across validation and commit. Each
mutation commits one atomic database batch. The current database interface does not
expose a read/write transaction closure or compare-and-swap. Applications with multiple
writer processes need external serialization. Direct database mutations bypass these
helper-level invariants, just as they bypass the app's label editing workflow.

## Commands

`BasePackage` supplies these commands to hosts implementing `LabelContext`. All payloads
are semantic `Value::Object` values, and accept optional `scope_id`.

| Command suffix (`semantic.base.labels.`) | Additional payload | Output |
| --- | --- | --- |
| `list` | none | List of label objects |
| `load` | `id`, optional `collection` | Assigned label objects |
| `add` / `remove` / `replace` | `id`, optional `collection`, `label_ids` list | Persisted selection |
| `save` | `label` object with ID and label attributes | Saved label object |
| `delete` | Label `id` | Null |

`collection` defaults to `entities`. `save` creates or updates by ID. The `Label` model
and `encode_labels` / `decode_labels` helpers provide the shared wire representation.

## UI

`/labels` provides the searchable hierarchy and label details form. The reusable
`semantic_ui_core::LabelEditor` takes an entity target, `on_close`, and optional
`on_changed(Vec<Label>)`. Keep it mounted until `on_close` fires. Save persists without
closing; Discard closes without saving; Escape, outside click, and Close persist first.
Errors leave the editor and draft open. `on_changed` runs only after a successful change.
Changing the target, collection, or active scope resets the editor draft and cancels
its local tasks. An already submitted server save can still finish for the original
target, but cannot update or close the new editor. Groups appear as nonselectable
headings; their child labels remain selectable.

The default entity action is `labels`; set `RenderSettings::enable_label_editor` to
`false` to hide it, or exclude the `labels` action for a specific entity card.
