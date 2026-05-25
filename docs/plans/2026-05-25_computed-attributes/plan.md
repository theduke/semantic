# Implementation Plan: Computed Attributes for Classes

## Overview

Add computed attributes to classes — attributes whose value is derived from an expression referencing other attributes of the same class (via `self`). The attribute's type is declared by `AttributeType.ty` as usual; the computed expression must produce a value assignable to that type.

### Key constraint

Computed attributes are **read-only in the schema layer**. They cannot participate in DML (inserts, updates, upserts). They are only materialized on read, when explicitly selected or when all attributes are requested.

---

## Phase 1: Data Model (`crates/data`)

### 1.1 Add `computed: Option<Expr>` to `ClassAttribute`

**File:** `crates/data/src/schema/class/class_attribute.rs`

Add the field:
```rust
/// If set, this attribute's value is computed from this expression
/// at read time rather than stored. The expression must reference
/// other attributes of the same class via `RefExpr::Identifier("self")`.
pub computed: Option<Expr>,
```

The `Expr` type from `crates/data/src/expr/core/expression.rs` already supports:
- `LiteralExpr` — for string literals like `"my_prefix-"`
- `BinaryExpr { op: Concat, left, right }` — string concatenation
- `CallExpr { callee: Callee::Name(["stringify"]), args }` — type-to-string conversion
- `FieldAccessExpr { target: Ref(Identifier("self")), field: "other" }` — referencing sibling attributes

**No changes to `Expr`, `RefExpr`, `BinaryOperator`, `Callee`, or any variant**.

### 1.2 Re-export

**File:** `crates/data/src/schema/class/mod.rs` — no change needed (auto-pub via `class_attribute` module).

**File:** `crates/data/src/schema/mod.rs` — no change needed, `ClassAttribute` already re-exported.

---

## Phase 2: Validation (`crates/db_core/src/validation.rs`)

### 2.1 New validation module: `validate_computed_attribute()`

Added to the existing validation infrastructure. A dedicated function that takes `(&Catalog, &ClassType, &ClassAttribute)` and validates:

| Rule | Description |
|---|---|
| **Self-only scoping** | `RefExpr::Identifier("self")` is only valid inside `computed` expressions. Error if `self` appears elsewhere. |
| **Field resolution** | `FieldAccess { target: Ref(Identifier("self")), field: "name" }` — resolve `"name"` against the declared attributes of this class (including inherited and extended). |
| **No self-referencing** | A computed attribute must not reference itself directly or transitively. |
| **No computed → computed cycles** | Build a dependency graph across all computed attributes of a class. Reject cycles. |
| **Type compatibility** | The expression's inferred type must be assignable to `AttributeType.ty`. |
| **No side-effects** | Only pure expressions allowed. Reject: `Query`, `Subquery`, `Lambda` (for now), mutation-adjacent calls. |
| **Builtin function registry** | Only registered builtin functions like `stringify` are allowed in `Callee::Name(...)`. |

#### Field resolution algorithm

```
fn resolve_self_field(catalog: &Catalog, class: &ClassType, field_name: &str) -> Result<&AttributeType>
  → walk class, then inherits chain, then extends
  → return the attribute if found, or error
```

#### Type inference for expressions (minimal — for `stringify` + `Concat`)

```
fn infer_expr_type(catalog: &Catalog, class: &ClassType, expr: &Expr) -> Result<Type>
  - Literal(string)  → String
  - Ref("self")      → Error (needs field access)
  - FieldAccess(self, field)  → resolve to attribute's type
  - Binary(Concat, left, right) → both must be String → String
  - Call(stringify, _)  → String
  - Call(unknown, _)    → Error
  - etc.
```

#### Cycle detection

Build an adjacency list: `computed_attr_a → {fields it references}`, same for the transitive closure. If any computed attribute references itself or forms a cycle, reject.

### 2.2 Hook into DDL validation

The validation runs as part of `apply_ddl_batch` / `CatalogBatchOperation::UpsertClass`. When a class is upserted, iterate its attributes. For each one with `computed: Some(expr)`, run the validation.

---

## Phase 3: Expression Semantics

### 3.1 The `self` identifier

`RefExpr::Identifier("self")` in the context of a `ComputedAttribute.expr` refers to the enclosing class instance.

- In expressions like `self.other_field`, it appears as:
  ```rust
  FieldAccessExpr {
      target: Expr::Ref(RefExpr::Identifier("self".to_string())),
      field: "other_field".to_string(),
  }
  ```
- The validator ensures `"self"` is always followed by a field access (i.e., `self.other_field`, never bare `self` in a typed context). Bare `self` as a boolean/null test is allowed (e.g., `isNull(self)` or similar) but can be deferred.

### 3.2 The `stringify` builtin function

Represented as `Callee::Name(["stringify"])`. Semantics:
- Takes exactly one positional argument.
- Returns `String`.
- Behavior: converts any scalar value to its string representation. The exact formatting follows standard semantics (numbers → decimal digits, booleans → `"true"`/`"false"`, null → `"null"`, UUID → canonical hex form, etc.).

**Registration**: Maintain a set of known builtin function names. For now: `{"stringify"}`. Future: `"coalesce"`, `"format"`, `"ifnull"`, etc.

### 3.3 `Concat` vs `Add` for string concatenation

`BinaryOperator::Concat` already exists. It is the correct operator for string concatenation.
- Both operands must be `String`-compatible.
- Result is `String`.
- The expression `'my_prefix-' + stringify(self.other_field)` uses `Concat`, not `Add`.

### 3.4 General expression constraints

- **No query expressions**: `Query`, `Subquery`, `Exists`, `In` with subquery are all rejected. Only scalar expressions allowed.
- **No lambda/functions**: `Lambda` expressions rejected.
- **Allowed**: Literals, Ref (self-only), FieldAccess, Binary, Unary, Call (builtins only), Cast, If/Case/Let (pure forms), List/Tuple/Map constructors.

---

## Phase 4: Downstream — Attribute Registration

### 4.1 Catalog: class attribute indexing

When the catalog builds a `ClassSchema` (in `Catalog::apply_batch`), it currently iterates `class.attributes` and builds the `attributes: BTreeMap<String, LocalAttrId>` mapping. This needs no structural change — computed and stored attributes look the same from the catalog's perspective (both map to a `LocalAttrId`).

However, the catalog needs a way to distinguish computed vs. stored attributes for materialization. Two approaches:

**Option A (recommended): Store `computed` on `ClassSchema`**
```rust
pub struct ClassSchema {
    pub lid: LocalClassId,
    pub names: NameSet,
    pub class: ClassType,
    pub attributes: BTreeMap<String, LocalAttrId>,
    pub computed_attributes: BTreeMap<String, Expr>,  // NEW: attr_id → expr
}
```

**Option B: Check `ClassAttribute.computed` at materialization time**
Look up `class.class.attributes.get(field_name).and_then(|a| a.computed.as_ref())`.

Option B is simpler and avoids duplicating state. Since the `ClassSchema` already holds the full `ClassType` in `class`, we can always reach into it. The tradeoff is a `BTreeMap::get` + field access at every materialization point, which is negligible.

**Decision: Use Option B** — reach into `ClassSchema.class.attributes[name].computed` at materialization time. Redundant caching only if benchmarks show a need.

### 4.2 Collection schema: computed fields in field_types

The `CollectionSchema.field_types` map is built from registered attributes. Computed attributes produce a type (from `AttributeType.ty`) just like stored ones. They are registered identically.

However, computed attributes should be flagged so the collection schema knows they are not writeable. A simple approach: add a `computed_fields: BTreeSet<String>` to `CollectionSchema`:

```rust
pub struct CollectionSchema {
    // ... existing fields ...
    /// Fields whose values are computed at read time and cannot be written.
    pub computed_fields: BTreeSet<String>,
}
```

This is populated during `build_collection_schema_for_lid` by scanning class attributes for `computed.is_some()`.

### 4.3 Filter computed attributes on persist (execution layer)

When persisting objects (insert, update, upsert), computed attributes must be **stripped from the input before it reaches storage**. This is separate from DDL-level validation — it happens in the execution layer (`db_core`'s plan execution / DML handlers).

**Rationale**: A client could send an object containing computed fields (e.g., from a previous read that injected them). These fields are not stored columns; writing them would either corrupt data or cause a schema violation.

**Implementation**: Before any write operation, the execution layer:

1. Looks up the collection's `computed_fields: BTreeSet<String>` from `CollectionSchema`.
2. Removes every key in the input object that matches a computed field name.
3. Proceeds with the stripped object.

**Key locations**:
- `BatchOperation::Upsert` — strip computed fields from the object before storing
- `InsertQuery` execution — strip computed fields from each row in `InsertSource::Objects`
- `UpdateQuery` execution — reject computed fields in `assignments` (updating a computed field is semantically wrong)
- `normalize_object_for_collection` — strip computed fields during normalization (defense-in-depth alongside the execution layer)

**Relevant error for update assignments to computed fields**:
```
cannot update field '{name}' — attribute is computed
```

### 4.4 Collection schema: DML write rejection

In `validate_object_fields` and related DML validation paths, any attempt to write a computed field should be rejected with a clear error ("field '{name}' is computed and cannot be written").

Relevant locations:
- `normalize_object_for_collection` — reject computed fields in input objects
- DML execution paths in `db_core` (future)

---

## Phase 5: Downstream — Materialization

### 5.1 When to compute

Computed attributes are materialized at read time. They must be injected whenever:

1. **All attributes are selected** (select `*` or no explicit projection). The query engine must evaluate every computed attribute and add it to the result.
2. **A computed attribute is explicitly selected**. Only that computed attribute is evaluated.
3. **A computed attribute appears in a query predicate or ORDER BY**. The computed value must be available for filtering/sorting.

### 5.2 The materialization pipeline

The materialization process converts a `ComputedAttribute.expr` (semantic data model `Expr`) into an executable form:

```
1. Resolve self → replace RefExpr::Identifier("self") with the actual object
2. Resolve FieldAccess(self, field) → read the field value from the object
3. Evaluate Call(stringify, arg) → convert value to string
4. Binary(Concat, left, right) → evaluate both, concatenate
5. Recursively evaluate the full Expr tree
```

#### Evaluation function (conceptual)

```rust
fn evaluate_computed_expr(
    obj: &Object,
    expr: &Expr,
) -> Result<Value, EvalError> {
    match expr {
        Expr::Literal(lit) => Ok(literal_value_to_value(&lit.value)),
        Expr::Ref(RefExpr::Identifier("self")) =>
            Err("bare self reference not allowed in computed expression"),
        Expr::FieldAccess(fa) => {
            let target = evaluate_computed_expr(obj, &fa.target)?;
            match target {
                Value::Object(o) => o.get(&fa.field).cloned()
                    .ok_or_else(|| EvalError::UnknownField(fa.field.clone())),
                _ => Err(EvalError::NotAnObject),
            }
        }
        Expr::Binary(bin) => {
            let left = evaluate_computed_expr(obj, &bin.left)?;
            let right = evaluate_computed_expr(obj, &bin.right)?;
            match bin.op {
                BinaryOperator::Concat => {
                    match (left, right) {
                        (Value::String(a), Value::String(b)) => Ok(Value::String(a + &b)),
                        (Value::String(a), other) => Ok(Value::String(a + &other.to_string())),
                        (other, Value::String(b)) => Ok(Value::String(other.to_string() + &b)),
                        _ => Err(EvalError::TypeMismatch),
                    }
                }
                _ => Err(EvalError::UnsupportedOp),
            }
        }
        Expr::Call(call) => {
            let evaluated_args: Vec<Value> = call.args.iter()
                .map(|arg| match arg {
                    CallArg::Positional(e) => evaluate_computed_expr(obj, e),
                    CallArg::Named { .. } => Err(EvalError::NamedArgsNotSupported),
                })
                .collect::<Result<Vec<_>, _>>()?;
            match call.callee {
                Callee::Name(ref name) if name == ["stringify"] => {
                    if evaluated_args.len() != 1 {
                        return Err(EvalError::WrongArgCount);
                    }
                    Ok(Value::String(value_to_string(&evaluated_args[0])))
                }
                _ => Err(EvalError::UnknownFunction),
            }
        }
        _ => Err(EvalError::UnsupportedExpr),
    }
}
```

### 5.3 Injection at read time

Three materialization paths:

**Path A: Query projection post-processing**
After the query engine fetches rows from storage, before returning:
1. For each row, check the class of the object.
2. If the projection is `*` (all fields) or explicitly includes a computed attribute, evaluate the computed expression.
3. Inject the computed value into the result object.

This is the cleanest approach — the storage layer is unchanged; computed attributes are a pure logical layer on top.

**Path B: Query-level expression rewrite (optimization)**
When a query explicitly selects a computed attribute, the query planner can rewrite the `Expr` to:
1. Resolve `self.field` references to the actual attribute field IDs.
2. Incorporate the resolved expression into the query plan as a virtual column.

This is an optimization for SQL backends and can be implemented later.

### 5.4 Serialization / export path

When serializing entities (e.g., JSON export, API responses), computed attributes must follow the same rules:
- If the serialization context requests all attributes, compute and inject.
- If only specific attributes are requested, compute only those that are computed and selected.
- Computed attributes are always serialized as regular fields (no metadata distinguishing them in the output).

---

## Phase 6: Migration & DDL

### 6.1 Existing DDL operations

`DdlOperation::UpsertClass` already carries a full `ClassType` with `BTreeMap<String, ClassAttribute>`. The new `computed` field is just an `Option<Expr>` on each `ClassAttribute` — it flows through automatically with no DDL changes.

### 6.2 Downstream DDL construction sites

All existing `ClassAttribute { ... }` constructions must add `computed: None`:

| File | Occurrences | Pattern |
|---|---|---|
| `crates/db_core/src/ddl.rs` | ~22 | `ClassAttribute { attribute, required, constraints, meta }` |
| `crates/db_core/src/validation.rs` | 1 (test) | Same pattern |
| `crates/db_kv/src/db.rs` | 1 | Same pattern |

Each gets `computed: None` added.

---

## Phase 7: File-by-file Change Summary

| # | File | Change |
|---|---|---|
| 1 | `crates/data/src/schema/class/class_attribute.rs` | Add `pub computed: Option<crate::expr::Expr>` field to `ClassAttribute` |
| 2 | `crates/db_core/src/validation.rs` | Add `validate_computed_attribute()` with field resolution, type inference, cycle detection, builtin registry |
| 3 | `crates/db_core/src/validation.rs` | Hook validation into DDL upsert path |
| 4 | `crates/db_core/src/ddl.rs` | Add `computed: None` to all `ClassAttribute { ... }` constructors (~22 sites) |
| 5 | `crates/db_kv/src/db.rs` | Add `computed: None` to the test `ClassAttribute { ... }` constructor (1 site) |
| 6 | `crates/db_core/src/catalog/schema.rs` | Add `computed_fields: BTreeSet<String>` to `CollectionSchema` |
| 7 | `crates/db_core/src/catalog/catalog.rs` | Populate `computed_fields` in `build_collection_schema_for_lid` by scanning class computed attributes |
| 8 | `crates/db_core/src/catalog/catalog.rs` | Reject writes to computed fields in `apply_batch` DML paths, populate `computed_fields` in `build_collection_schema_for_lid` |
| 9 | `crates/db_core/src/validation.rs` | Strip computed fields in `normalize_object_for_collection` (defense-in-depth), reject update assignments to computed fields |
| 10 | `crates/db_core/src/plan/execute.rs` (or equivalent DML handlers) | **Execution layer: strip computed fields from input before any persist** — `Upsert`, `Insert`, and `Update` paths must filter out computed attributes from the object before writing to storage |
| 11 | `crates/db_core/src/plan/execute.rs` (or equivalent) | Implement `evaluate_computed_expr()` — the runtime evaluator for computed expressions |
| 12 | `crates/db_core/src/plan/execute.rs` (or equivalent) | Inject computed attributes in query results: post-process each row, evaluate expressions, inject into object |
| 10 | `crates/db_core/src/plan/execute.rs` (or equivalent) | Implement `evaluate_computed_expr()` — the runtime evaluator for computed expressions |
| 11 | `crates/db_core/src/plan/execute.rs` (or equivalent) | Inject computed attributes in query results: post-process each row, evaluate expressions, inject into object |

---

## Phase 8: Testing Strategy

### Unit tests in `crates/db_core`

| # | Test | Description |
|---|---|---|
| 1 | `computed_attribute_self_field_resolves` | Class with `computed: Some(FieldAccess(self, "title"))`. Verify `title` attribute resolves. |
| 2 | `computed_attribute_string_concat` | `'prefix-' + stringify(self.other)`, attribute type String. Verify type inference succeeds. |
| 3 | `computed_attribute_type_mismatch_rejected` | Expression returns Number but attribute type is String. Verify validation error. |
| 4 | `computed_attribute_self_cycle_detected` | attr_a references attr_b, attr_b references attr_a. Verify cycle error. |
| 5 | `computed_attribute_self_reference_rejected` | attr_a references itself. Verify error. |
| 6 | `computed_attribute_inherited_field_reference` | Computed in child class references an attribute from parent class via `self.parent_field`. Verify resolution. |
| 7 | `computed_attribute_no_side_effects` | Query/Subquery/Lambda in computed expression. Verify rejection. |
| 8 | `computed_attribute_unknown_builtin_rejected` | `unknown_func(self.x)` in computed expression. Verify rejection. |
| 9 | `computed_attribute_not_writable` | DML upsert with computed field in input. Verify rejection. |
| 10 | `computed_attribute_materialized_on_read` | Store object, read back with all fields. Verify computed value is present and correct. |
| 11 | `computed_attribute_materialized_when_selected` | Explicitly select a computed attribute. Verify it is computed. |
| 12 | `computed_attribute_stored_attribute_not_selected` | Select non-computed attributes. Verify computed attributes are NOT injected. |

### Integration tests in `crates/db_test`

Full round-trip tests through the DDL → insert → select pipeline, verifying computed attributes appear in results.

---

## Appendix A: Example Expression Serialization

For the schema DSL (future, not in this plan), a computed attribute definition could look like:

```
attribute display_name: String {
    computed: "my_prefix-" + stringify(self.other_field)
}
```

This serializes to:

```rust
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

Every `Expr` variant used here already exists in the codebase.

---

## Appendix B: Future Considerations (out of scope for this plan)

| Topic | Description |
|---|---|
| **SQL backend pushdown** | For SQL-based storage, compiled computed attributes into SQL expressions (e.g., `CONCAT('prefix-', CAST(other_field AS VARCHAR))`). |
| **Caching computed values** | Optionally materialize and cache computed values in a secondary index for fast filtering/sorting. |
| **`coalesce` builtin** | `coalesce(self.a, self.b, 'default')` — return the first non-null value. |
| **`format` builtin** | `format("Hello {0}!", self.name)` — template-style string formatting. |
| **Cross-class computed** | Computed expressions referencing attributes from related classes via relationship traversal. |
| **Lambda in computed** | Allowing small lambda expressions for map/reduce over collection attributes. |
