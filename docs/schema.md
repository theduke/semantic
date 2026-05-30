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
| `Ref { name, args }` | Reference to a named type definition |
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
    pub computed: Option<Expr>,    // computed at read time from other attrs
    pub constraints: Vec<Constraint>,
    pub meta: Meta,
}
```

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
