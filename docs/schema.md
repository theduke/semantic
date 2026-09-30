# Schema System

## Overview

The schema system in `crates/data/src/schema/` is a complete type definition language
designed for composable, service-oriented data systems. It is architected with strong
inspiration from the **WebAssembly Component Model**, mapping worlds, interfaces,
resources, and imports into a unified type schema.

The hierarchy:

```
SchemaDoc
├── imports              — external schema dependencies
├── defs                 — standalone type definitions (TypeDef)
├── attributes           — globally shared attributes
├── classes              — data entity definitions
├── contracts            — root-level contract/world definitions
├── root_module          — root module (optional)
├── modules              — named sub-modules, each containing:
│     ├── types          — type definitions
│     ├── attributes     — shared attributes
│     ├── classes        — data classes
│     ├── interfaces     — bare interface definitions
│     └── contracts      — contract/world definitions
└── packages             — deployable units, each containing:
      └── modules        — with a distinguished root module
```

---

## Naming Convention

All globally unique identifiers use **dot-separated** or **colon-separated** qualified
names. Both separators appear in existing usage, with dots more common for hierarchical
grouping.

| Pattern | Example | Usage |
|---|---|---|
| `scope.entity.name` | `shared.blog.title` | Attribute, class, and type IDs |
| `scope:entity:name` | `shared:test:kind` | Also used in IDs |

The `id` field holds the globally unique qualified identifier; the `name` field holds
the human-readable short name.

```rust
ClassType  { id: "shared.blog.post",  name: "BlogPost",  ... }
Attribute  { id: "shared.blog.title", name: "title",     ... }
```

---

## Core Types

### TypeKind — the type universe

Every type is one variant of the `TypeKind` enum, wrapped in `Type` (which adds
constraints and annotations).

```rust
pub struct Type {
    pub kind: TypeKind,
    pub constraints: Vec<Constraint>,
    pub annotations: Vec<Annotation>,
}
```

**Primitives:**

| Variant | Payload | Description |
|---|---|---|
| `Any` | — | Any value |
| `Never` | — | Uninhabited type |
| `Unknown` | — | Unknown type (placeholder) |
| `Null` | — | Null/nil |
| `Bool` | — | Boolean |
| `Char` | — | Unicode character |
| `Number` | `{ format, bounds }` | Numeric value (int/float/bigint/rational/decimal) |
| `String` | `{ format, normalization }` | String with optional format constraints |
| `Bytes` | `{ encoding }` | Binary data with encoding info |
| `Temporal` | `{ kind }` | Date, time, datetime, duration |
| `Uuid` | — | UUID |
| `IpAddr` | `{ version }` | IP address |
| `Json` | — | Arbitrary JSON |
| `Opaque` | `{ description }` | Opaque blob with human description |
| `Extension` | `{ namespace, name, payload }` | External/custom type backed by plugin logic |

**Collections:**

| Variant | Payload |
|---|---|
| `Optional<T>` | Single element type |
| `Array<T>` | Fixed-size array with length |
| `List<T>` | Variable-length list |
| `Tuple<T₁..Tₙ>` | Fixed-position tuple |
| `Map<K, V>` | Key-value map |
| `Set<T>` | Unique-value set |

**Structured types:**

| Variant | Description |
|---|---|
| `Record { fields }` | Anonymous struct with named fields |
| `Attribute { id, name, ty }` | A single named, typed attribute (reusable across classes) |
| `Class { id, name, attributes, inherits, extends }` | A named data entity (like a DB table / ORM model) |
| `Union<T₁..Tₙ>` | One of several types (untagged) |
| `Intersection<T₁..Tₙ>` | All of several types simultaneously |
| `Variant { cases }` | Tagged union (one of several named cases, each with optional payload) |
| `Enum { variants }` | Named constant values |
| `Result<Ok, Err>` | Success-or-error type |

**Behavioral types:**

| Variant | Description |
|---|---|
| `Function { params, results, throws, async_fn }` | A callable function signature |
| `Interface { methods }` | A group of related named methods |
| `Handle { interface, mode: Own/Borrow }` | A resource handle with ownership semantics |
| `Stream { element, end }` | An async stream of values |

**Meta types:**

| Variant | Description |
|---|---|
| `Named { name, args }` | Application of a named type definition |
| `Ref { target, on_delete }` | Stored entity-ID reference with foreign-key behavior |
| `Extension { namespace, name, payload }` | External/plugin-backed type |

### TypeDef — a named type definition

```rust
pub struct TypeDef {
    pub name: TypeName,          // qualified name, e.g. "shared.blog.status"
    pub module: Option<String>,  // which module this belongs to
    pub params: Vec<TypeParam>,  // generic type parameters
    pub ty: Type,                // the actual type
    pub visibility: Visibility,  // Public | Internal | Private
    pub meta: Meta,
}
```

`TypeName` is currently a type alias for `String`.

### Surface types vs. lowered data types

`TypeKind` is the *surface* type language. Every kind can be persisted as a
definition (contracts, interfaces and function signatures need all of them),
but only a subset can be inhabited by stored `Value`s. That subset is
`semantic_data::schema::lowered::DataType`: a derived, never-persisted form
produced by `lower_type` / `lower_type_def`. Lowering resolves `Named` types
through a `TypeResolver`, inlines them (recursive definitions become a
`Recursive(name)` back-reference once a list, map, tuple, record field or
variant payload separates the cycle), and fails with a typed `LowerError`
naming the offending kind and its path, e.g.
`attribute 'x' list item: function types cannot be stored`.

| Surface kind | Lowered disposition |
|---|---|
| `Any`, `Null`, `Bool`, `Char`, `String`, `Bytes`, `Uuid`, `IpAddr`, `Json` | Same kind |
| `Number` `Int`/`UInt` up to 128 bits, `Float` `F16`/`F32`/`F64`, `Unspecified` | `Number` |
| `Number` `I256`/`U256`, `F80`/`F128`, `Decimal*` floats, `BigInt`, `BigUInt`, `Decimal`, `Rational`, `Complex` | Error |
| `Temporal` `Date`/`Time`/`DateTime`/`Duration` | `Temporal` |
| `Temporal` `Timestamp` | `Temporal(DateTime)` (documented exception: backends store timestamps as date-times) |
| `Temporal` `Period`/`Instant` | Error |
| `Optional`, `List`, `Tuple`, `Map`, `Record`, `Union`, `Enum`, `Variant`, `Ref` | Same kind, children lowered |
| `Array` / `Set` | `List` carrying `length` / `distinct` |
| `Result` | Externally tagged `Variant` with `ok` / `err` cases |
| `Intersection` | Merged `Record` if every part lowers to a record, else error |
| `Named` | Resolved and inlined; reference-site constraints are appended. Unresolved names, generic arguments and generic definitions are errors, as are alias cycles without an intervening data constructor |
| `Attribute` (inline) | The attribute's value type with its constraints appended |
| `Class` (inline) | `Record` keyed by canonical attribute ID including inherited attributes; strict classes are closed, others open. Unresolved attributes are skipped (same as stored-value validation) |
| `Extension` | Kept as an opaque `Extension` (values are not interpreted) |
| `Function`, `Interface`, `Handle`, `Stream`, `Never`, `Unknown`, `Opaque` | Error |

**Enforcement.** The catalog tags every `TypeDefSchema` with its lowered
form (`TypeDefData::Storable(DataType)`) or the lowering error
(`TypeDefData::Unstorable`, e.g. interface-only definitions). Tags are
recomputed after every schema change. `Catalog::apply_batch` — the path used by
DDL batches and package migrations — lowers against the post-batch catalog, so
definitions may reference types registered later in the same batch, and
rejects any attribute or record type that does not lower with
`CatalogError::UnstorableType`. This includes attributes whose `Named` type
points at an interface-only definition, and batches that redefine a referenced
type as interface-only. Classes are covered through their attributes and other
type definitions are only tagged.

Existing databases are never rejected: loading a catalog
(`from_storage_snapshot` / `from_stored_rows`) only tags definitions, and a
batch tolerates a data definition that already failed to lower before the
batch as long as it is left unchanged. The direct `Catalog::upsert_*`
registration helpers remain unvalidated (as before) but keep the tags current.

---

## Attributes and Classes

### AttributeType

A reusable, globally identified attribute definition. Attributes are the primitive
building blocks of data entities.

```rust
pub struct AttributeType {
    pub id: String,        // e.g. "shared.blog.title"
    pub name: String,      // short name, e.g. "title"
    pub ty: Type,          // the value type
    pub constraints: Vec<Constraint>,
    pub meta: Meta,
}
```

### ClassType

A class is a named data entity composed of attributes. It represents a logical
record or database entity type.

```rust
pub struct ClassType {
    pub id: String,
    pub name: String,
    pub inherits: Option<ClassRef>,              // single base class
    pub extends: Vec<ClassRef>,                   // mixin-style extensions
    pub creatable_in_ui: Option<bool>,
    pub include_in_ui_listings: Option<bool>,
    pub attributes: BTreeMap<String, ClassAttribute>,  // attribute -> per-class config
    pub constraints: Vec<ClassConstraint>,
    pub meta: Meta,
}
```

A `ClassAttribute` wraps an `AttributeRef` with per-class overrides:

```rust
pub struct ClassAttribute {
    pub attribute: AttributeRef,   // references an AttributeType by id
    pub required: bool,
    pub ui_order: Option<u32>,     // optional generated UI ordering hint
    pub computed: Option<Expr>,    // computed at read time from other attrs
    pub default: Option<Expr>,     // per-class field default expression
    pub constraints: Vec<Constraint>,
    pub meta: Meta,
}
```

The entity create form initializes fields from literal `default` expressions.
Other expressions remain part of the class schema for consumers that can
evaluate them.

`semantic:ui:include_in_listings` controls whether instances of a class appear in
generic UI listings such as `/browse` and collection pages. Only `false` excludes
them; `true`, `null`, and an omitted attribute include them. This setting applies
to each class independently. It does not restrict explicit SQL queries, entity
detail pages, or dedicated views such as the directory browser. Relation entities
and their subclasses are separately excluded from default entity browsing.

`semantic:ui:creatable_in_ui` separately controls whether generic creation forms
offer a class.

### Relations

`RelationType` models connections between classes (e.g., foreign key relationships).

---

## The Component Model: Contracts

The schema is architected around the **WebAssembly Component Model** paradigm.
`Contract`, `InterfaceType`, and `HandleType` together form a composable service
boundary system analogous to Wasm worlds, interfaces, and resources.

### Contract — a service boundary (Wasm "world")

A `Contract` is a **named, self-contained service declaration** — the schema analogue
of a Wasm component world. It bundles:

- The **data types** it operates on
- The **functions** it provides
- The **interfaces** it exposes
- The **attributes** and **classes** it uses
- The **constants** it defines

```rust
pub struct Contract {
    pub name: String,
    pub constants:  BTreeMap<String, ContractConstant>,
    pub types:      BTreeMap<TypeName, TypeDef>,
    pub functions:  BTreeMap<String, ContractFunction>,
    pub attributes: BTreeMap<String, AttributeType>,
    pub classes:    BTreeMap<String, ClassType>,
    pub interfaces: BTreeMap<String, ContractInterface>,
    pub meta: Meta,
}
```

A contract is the **export boundary** — everything defined inside it is what the
contract provides to consumers. Its functions are the remote-callable operations.

A contract can live at two levels:
- **Root-level**: directly on `SchemaDoc.contracts`
- **Module-scoped**: inside a `Module.contracts`

#### ContractFunction

A named, typed function — the RPC operation:

```rust
pub struct ContractFunction {
    pub name: String,
    pub signature: FunctionType {
        pub params: Vec<FunctionParam>,      // named/typed parameters
        pub results: Vec<Type>,              // return type(s)
        pub throws: Option<Box<Type>>,       // error type
        pub async_fn: bool,
    },
    pub meta: Meta,                          // annotations, docs, tags, etc.
}
```

#### ContractInterface

A named interface wrapping a group of methods:

```rust
pub struct ContractInterface {
    pub name: String,
    pub interface: InterfaceType {
        pub methods: Vec<InterfaceMethod>,
    },
    pub meta: Meta,
}
```

### Imports — external dependencies

`SchemaImport` declares a dependency on an external schema:

```rust
pub struct SchemaImport {
    pub namespace: String,
    pub location: String,
    pub version: Option<SchemaVersion>,
    pub optional: bool,
}
```

Imports make external types referenceable within the schema via their namespace
prefix. Import resolution (loading the remote schema and validating cross-references)
is not yet implemented.

### Handles — resources with ownership (Wasm "resource")

`HandleType` models a **typed resource reference** with ownership semantics,
directly analogous to Wasm component model handles:

```rust
pub struct HandleType {
    pub interface: TypeRef,     // which interface defines operations on this handle
    pub mode: HandleMode,       // Own | Borrow
}
```

| Mode | Semantics |
|---|---|
| `Own` | The handle owns the resource; must be dropped exactly once |
| `Borrow` | Temporary reference; must not outlive the source |

A handle links to an `InterfaceType`, which defines the methods that can be called
on the resource. This is the mechanism for passing typed capabilities across
component/service boundaries.

### Visibility

```rust
pub enum Visibility { Public, Internal, Private }
```

Currently only used on `TypeDef`, but architecturally applicable to contracts,
functions, interfaces, and classes.

---

## Metadata and Annotations

Every definition carries `Meta` — a rich metadata block:

```rust
pub struct Meta {
    pub title: Option<String>,
    pub description: Option<String>,
    pub id: Option<String>,
    pub deprecated: Option<Deprecation>,
    pub aliases: Vec<String>,
    pub examples: Vec<LiteralValue>,
    pub tags: Vec<String>,
    pub docs_url: Option<String>,
    pub annotations: BTreeMap<String, String>,   // arbitrary key-value decorations
}
```

Additionally, the `Type` node itself carries structured annotations:

```rust
pub struct Annotation {
    pub key: String,
    pub value: AnnotationValue,  // Bool | Number | String | List | Map
}
```

---

## RPC Addressing

For a server that fulfills contract functions, the canonical address for a
callable operation is:

```
module:contract:function
```

| Segment | Source | Description |
|---|---|---|
| `module` | `Module.name` | The module scope |
| `contract` | `Contract.name` | The service/world boundary |
| `function` | `ContractFunction.name` | The callable operation |

**Examples:**

| Address | Resolves to |
|---|---|
| `blog:user-service:create_user` | `modules["blog"].contracts["user-service"].functions["create_user"]` |
| `inventory:stock:check_availability` | `modules["inventory"].contracts["stock"].functions["check_availability"]` |

For root-level contracts (on `SchemaDoc.contracts` directly), the shorter form
`contract:function` may be used when unambiguous.

---

## Current Implementation Status

| Concept | Status |
|---|---|
| Type system (TypeKind) | ✅ Complete |
| Type definitions (TypeDef) | ✅ Complete |
| Attributes and classes | ✅ Complete (with DB migrations) |
| Imports (SchemaImport) | 🟡 Schema defined, resolution not implemented |
| Contracts (worlds) | 🟡 Schema defined, no fulfillment/runtime |
| Contract functions | 🟡 Schema defined, no dispatch/runtime |
| Handles (resources) | 🟡 Schema defined, not wired into backend |
| Visibility | 🟡 Schema defined, only wired into TypeDef |
| Extension types | 🟡 Schema defined, no plugin system |
| Inter-module references | 🟡 Schema supports via TypeRef, no validation |
| Composition engine | ❌ Not built |
| Codegen from contracts | ❌ Not built |

The schema system is **schema-first**: the complete type definition language is in
place, but the runtime layers (import resolution, contract fulfillment, handle
passing, component composition) have not yet been built.
