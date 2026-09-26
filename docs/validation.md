# Stored-value validation

Existing databases retain legacy write validation until explicitly activated. Activation is persistent and applies to all managed collections in that database; it does not enable file garbage collection.

1. Call `Db::validation_preflight()` or RPC `semantic.db.validation.preflight` with `{scope_id?}`. This read-only, revision-checked report contains the first violation for each affected row: `{collection,id,error}`. Repeat after repairs until the report is empty. Unsupported enforcement declarations return `unsupported_constraint {kind}` even when no rows use them.
2. Repair legacy rows with normal writes or package data migrations. Preflight never inserts defaults or rewrites data. Supply missing stored attributes, repair invalid values, and create or relink missing targets.
3. Call `Db::activate_validation()` or RPC `semantic.db.validation.activate` with `{scope_id?}`. It repeats preflight and commits the activation marker and reverse-reference backfill together, refusing violations or a concurrent data change. The APIs are available on memory, redb, and managed PostgreSQL backends.

The SDK exposes typed `commands.validationPreflight` and `commands.validationActivate` through `client.invoke`. Activation returns no value. Preflight violations and `validation_failed` RPC error data contain `class`, canonical `attribute`, `path`, `rule`, `expected`, and `actual`. Paths use the existing `FieldPath` representation, for example `[{field:"example:payload"},{field:"members"},{index:0}]`. Map entries add an entry index followed by `key` or `value`.

Active writes apply defaults and remove nullish optional stored fields before recursively checking required attributes, records/classes, enum/union alternatives, lists, fixed arrays, tuples, maps, and references. Required computed attributes need no stored value. Unions try every alternative; only successful alternatives contribute validated references. Conservative reverse-reference candidates ensure deleting or retyping a target revalidates surviving owners, including nested values. Same-batch forward references and coordinated deletion are validated against the final state.

Enforced constraints are `Min`, `Max`, `Length`, `Pattern`, `Prefix`, `Suffix`, `MinItems`, `MaxItems`, `MinProperties`, `MaxProperties`, and `RequiredFields`. Literal/expression defaults and transport annotations retain their preparation/metadata roles. Other enforcement constraints, including multi-field expressions and declaration-level uniqueness, are explicitly unsupported; ordinary database unique indexes continue to operate independently.

Entity references are declared as types:

```json
{"ref":{"name":"example:Person","on_delete":"restrict"}}
```

Every `Ref` stores an entity's primary ID and enforces existence. A target, when present, must resolve directly to a registered class; subclasses are accepted. `target: null` accepts any entity class. `on_delete: restrict` rejects target deletion while a surviving owner refers to it. `on_delete: cascade` deletes the owner, transitively and atomically. Builtin relation `from` and `to` use an unrestricted target with cascade behavior, so deleting either endpoint deletes the relation row.

`Named { name, args }` is separate: it applies a named schema `TypeDef` and does not store an entity ID.

After activation, class/attribute updates and package migrations validate the final existing data under the new catalog and backfill reverse references atomically. A schema change that would invalidate stored rows is rejected; package migrations can repair affected rows in the same transaction. There is no implicit activation or automatic repair on reopen.
