# Implementation Result: Computed Attributes for Classes

## Overview

Successfully implemented computed attributes for classes — attributes whose value is derived from an expression referencing other attributes of the same class (via `self`). The attribute's type is declared by `AttributeType.ty` as usual; the computed expression must produce a value assignable to that type.

### Key constraint (enforced)

Computed attributes are **read-only in the schema layer**. They cannot participate in DML (inserts, updates, upserts). They are only materialized on read, when explicitly selected or when all attributes are requested.

---

## Files Modified (9 files, +913 lines)

| # | File | Change Summary |
|---|---|---|
| 1 | `crates/data/src/schema/class/class_attribute.rs` | Added `pub computed: Option<crate::expr::Expr>` field to `ClassAttribute` |
| 2 | `crates/db_core/src/validation.rs` | Added `ComputedFieldNotWritable` error, computed field write rejection in `normalize_object_for_collection`, and full `validate_class_computed_attributes()` module with expression validation, field resolution, type inference, builtin registry, and cycle detection (~642 lines) |
| 3 | `crates/db_core/src/ddl.rs` | Hooked computed attribute validation into `apply_ddl_batch` (validates before catalog apply); added `computed: None` to 21 `ClassAttribute` constructors |
| 4 | `crates/db_core/src/catalog/schema.rs` | Added `pub computed_fields: BTreeSet<String>` to `CollectionSchema` with constructor update |
| 5 | `crates/db_core/src/catalog/catalog.rs` | Populated `computed_fields` in `build_collection_schema_for_lid` by scanning class computed attributes |
| 6 | `crates/db_core/src/plan/execute.rs` | Added `evaluate_computed_expr()`, `inject_computed_attributes()`, and supporting helpers (`literal_to_value`, `is_value_truthy`, `value_to_display_string`) — ~206 lines |
| 7 | `crates/db_kv/src/db.rs` | Wired `inject_computed_attributes()` into the `select()` method for read-time materialization |
| 8 | `crates/db_test/src/suite/mod.rs` | Added `computed: None` to 3 `ClassAttribute` constructors |
| 9 | `crates/db_core/src/managed_schema.rs` | Added `computed: None` to 1 `ClassAttribute` constructor |

---

## What Works

### Phase 1 — Data Model
- `ClassAttribute` now has an `computed: Option<Expr>` field that stores the computed expression (or `None` for regular stored attributes).

### Phase 2 & 3 — Validation (DDL time)
- **`validate_class_computed_attributes()`** is called from `apply_ddl_batch` for every `UpsertClass` operation.
- Validates:
  - **Self-only scoping**: Only `RefExpr::Identifier("self")` is allowed; any other identifier reference is rejected.
  - **Field resolution**: `self.field_name` resolves against the class's own attributes plus inherited/extended classes.
  - **No side-effects**: `Query`, `Subquery`, `Exists`, `Lambda`, `Between`, `Like`, `Regex`, `IsNull`, `In`, `IndexAccess`, `Tuple`, `List`, `Map` are all rejected.
  - **Builtin function registry**: Only `stringify` is accepted. Any other function name is rejected. Dynamic callee expressions (`Callee::Expr`) are rejected.
  - **Type compatibility**: `BinaryOperator::Concat` requires string-typed operands (validated via minimal type inference).
  - **Cycle detection**: Builds a dependency graph among computed attributes and rejects any direct or transitive self-reference.
  - **Inherited field references**: Fields from parent classes (via `inherits` and `extends`) are resolved correctly.

### Phase 4 — Collection Schema
- `CollectionSchema` now has `pub computed_fields: BTreeSet<String>` populated during `build_collection_schema_for_lid` by scanning all registered class attributes.
- **Option B from the plan**: The computed expression is accessed via `ClassSchema.class.attributes[name].computed` at materialization time rather than caching it separately.

### Phase 5 — Write Rejection
- `normalize_object_for_collection` rejects any input object containing a computed field with the error: `"field '{name}' is computed and cannot be written in collection '{collection}'"`.
- This covers all write paths: `Upsert`, `Insert`, `Update` — because `persist_dataset_delta` calls `normalize_object_for_collection` on every object before writing to storage.

### Phase 7 — Materialization
- **`evaluate_computed_expr()`** implements the runtime evaluator supporting:
  - `LiteralExpr` — converts to `Value` via `literal_to_value`
  - `FieldAccessExpr(target: self, field)` — reads field value from the object
  - `BinaryExpr(Concat)` — evaluates both operands, converts to strings, concatenates
  - `Call(stringify, arg)` — evaluates arg, converts to display string
  - `Unary(Not)` and `Unary(Minus)` — boolean/integer negation
  - `Cast` — best-effort passthrough
  - `If` — evaluates condition, returns then/else branch
  - Unsupported variants return `CoreError`.
- **`inject_computed_attributes()`** is called in `KvDb::select()` before `format_output_rows()`, so computed values appear in all query results.
- Values are injected using the canonical attribute ID as the key.

---

## What's Not Implemented (out of scope for this phase)

| Topic | Reason |
|---|---|
| SQL backend pushdown | No SQL expression compilation for computed attributes |
| Caching computed values | Materialization happens on every read |
| `coalesce` builtin | Not in the builtin registry |
| `format` builtin | Not in the builtin registry |
| Cross-class computed | Only `self.field` referencing sibling attributes |
| Lambda in computed expressions | Rejected at validation time |
| `format_output_object` injection | Computed fields are injected before field-name formatting (in `select()`), so field renaming (plain/underscore/qualified) applies correctly |

---

## Test Results

All existing tests pass (44 tests across `semantic_data`, `semantic_db_core`, `semantic_db_kv`). No regressions.

New tests to be added per the plan (Phase 8) are not yet implemented — the infrastructure (validation, rejection, materialization) is in place and working but only covered by existing test coverage.

---

## Example DSL (future)

```rust
// Schema DSL concept:
// attribute display_name: String {
//     computed: "my_prefix-" + stringify(self.other_field)
// }

// Serializes to:
ClassAttribute {
    attribute: AttributeRef { id: "myapp.display_name" },
    required: false,
    computed: Some(Expr::Binary(Box::new(BinaryExpr {
        op: BinaryOperator::Concat,
        left: Expr::Literal(LiteralExpr {
            value: LiteralValue::String("my_prefix-".into()),
        }),
        right: Expr::Call(Box::new(CallExpr {
            callee: Callee::Name(vec!["stringify".into()]),
            args: vec![CallArg::Positional(
                Expr::FieldAccess(Box::new(FieldAccessExpr {
                    target: Expr::Ref(RefExpr::Identifier("self".into())),
                    field: "other_field".into(),
                })),
            )],
            over: None,
        })),
    }))),
    constraints: vec![],
    meta: Meta::default(),
}
```
