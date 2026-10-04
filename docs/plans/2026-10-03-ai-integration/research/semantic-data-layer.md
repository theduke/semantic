# Semantic data layer: how to define and persist a new domain

Date: 2026-10-03. Scope: research only (no code changed). Purpose: inform the
design of (a) a reusable agent data-model crate whose types are "semantic data
based type definitions" and (b) an orchestration crate that persists agent
sessions/threads/turns/events in the Semantic DB.

All paths are relative to the repo root unless absolute. Line numbers are
omitted on purpose (the working tree has unrelated uncommitted dxgraph/ui
changes); grep for the quoted identifiers.

---

## 0. Executive summary

1. **A "domain" in Semantic is a `Package`** (`semantic_data::schema::Package`):
   a current-state `root` module (+ optional dependency modules) **plus an
   append-only list of `Migration`s**. The DB stores each applied migration
   verbatim and refuses to start (`MigrationMismatchPolicy::Fail`, the default)
   if a package presents a different definition of an already-applied
   migration. Package registration is `Db::upsert_package`; it is idempotent
   and cheap when nothing changed.
2. **Packages are not limited to `crates/base`.** Existing precedents live in
   three places: `crates/data` (`semantic.filestore`, `semantic.jobs`,
   `semantic.plugin`, `semantic.import`, `semantic.query`, planned
   `semantic.vdb`), `crates/base` (`semantic.base` plus the optional
   `semantic.comments` and `semantic.tasks`), and core migrations in
   `crates/db_core/src/ddl.rs`. A new `semantic.agent` package can live in its
   own crate; nothing in the DB or app requires it to be in `base`/`data`.
   AGENTS.md's "core (`crates/data`) and base (`crates/base`) must change only
   via migrations" applies to those packages; a brand-new package in a new
   crate follows the same discipline (its migrations are persisted in user DBs
   too) but does not touch core/base migrations.
3. **Rust types are mapped to schema by derives in `crates/macros`**
   (`SemanticType`, `IntoValue`, `FromValue`, `Class`), re-exported from
   `semantic_data`. **Important gap:** `#[derive(Class)]` derives the *codec*
   (struct <-> `Object` keyed by qualified attribute ids) and a *record
   `Type`*; it does **not** generate the `ClassType`/`AttributeType`
   definitions or the migration. Those are written by hand (see
   `crates/data/src/jobs.rs::package()`), and a test asserts the codec and the
   hand-written class agree. A schema-generating derive is a possible addition
   but is a core-layer change that needs sign-off (section 11).
4. **Class instances are keyed by fully qualified attribute ids**
   (`semantic:jobs:job:status`), exactly like DB query results; builtins `id`
   and `type` stay plain. **Record** types (RPC payloads, record-valued
   attributes) use plain field names.
5. **Persistence primitives that matter for agent streaming:**
   * Writes are `Batch { operations }` (atomic): `Create` (fails if exists),
     `Upsert` (whole-row replace), `DeleteById(s)`, predicate `Update`/`Delete`.
     There is **no append/patch-in-place of a field**, **no per-entity CAS**
     (conflict detection is whole-DB revision), and the app-facing
     `SemanticDb` facade does **not** expose interactive transactions.
   * Reads: SQL/PRQL/AST queries; point `get`; keyset pagination; equality,
     range, composite, partial and full-text indexes.
   * **Change feed exists only inside `db_core` embedded engines**
     (`Backend::subscribe_changes`, bounded broadcast, no replay) and is **not
     exposed through `SemanticDb`, RPC, or the UI**. Postgres has none.
   * **Server-to-client typed streaming exists** (`RpcStreamCommand`,
     `StreamOf<T, End>`, `RpcClient::invoke_stream`, wasm + native + embedded
     UI clients implement `invoke_interface`) but no production command uses it
     yet (only tests).
6. **Jobs (`crates/jobs`) are the wrong foundation for agent sessions**:
   inputs/outputs/checkpoints are runtime-only, no restart/resume, one global
   concurrency limit per scope (default 4), progress = completed/total.
   Reuse its *patterns* (interrupted-on-startup reconciliation, cancellation
   tokens, coalesced progress persistence) and optionally mirror a turn as a
   job *record* only if the 4-slot limit is solved. Recommendation: separate
   supervisor in the orchestration crate.
7. **Tasks (`semantic.tasks`) is a finished optional package** with
   status/priority/progress/parent and generic comments via an
   `entity_comment` relation. Agent threads should link to tasks through a
   generic `Ref`/relation (not a hard class dependency) and may reuse
   `MainContent` for rich prompts.
8. **High-frequency appends**: store the stream as many small immutable event
   rows in a dedicated collection with a unique composite `(thread, seq)`
   range index, coalesce token deltas (100-250 ms windows) before committing,
   and put large blobs in the filestore. Details in section 10.

---

## 1. `crates/data`: value, entity and ident model

### 1.1 `Value` and `Object`

`crates/data/src/value/val.rs`:

```rust
pub enum Value {
    Void, Null, Bool(bool),
    I8..I128, U8..U128, F32(OrderedF32), F64(OrderedF64),
    Uuid(Uuid), IpAddr(IpAddr),
    Duration(Duration), Time(Time), Date(Date), DateTime(DateTime),
    Bytes(bytes::Bytes), String(String),
    List(Vec<Value>), Map(Map), Object(Object),
    Variant(Box<VariantValue>),
}
pub const MAX_VALUE_DEPTH: usize = 128; // enforced by encoders and decoders
```

* `Object` is a `BTreeMap<String, Value>` newtype (`value/obj.rs`).
* `Value` is `Eq + Ord + Hash` (total order; see the deferred numeric-compare
  caveat in the DB memory note: mixed-width ints compare by enum rank, and SQL
  literals are `I64`; use the same integer width on both sides of a comparison
  and index, e.g. always `u64` for `seq`).
* There are canonical byte (`docs/spec/data-hash.md`) and typed-JSON
  (`value/serde/typed.rs`) encodings; JSONL export in `docs/formats/exports.md`.

### 1.2 Entities and ids

* An **entity** is an `Object` stored in a **collection** under a string
  `id`. Builtins (`crates/data/src/builtin.rs`):

  ```rust
  pub const ATTR_ID: &str = "id";
  pub const ATTR_TYPE: &str = "type";           // the class id
  pub const DEFAULT_COLLECTION: &str = "entities";
  ```
* The default collection `entities` is created by core migrations as
  `CollectionKind::Polymorphic` + `IntegrityMode::StrictRegisteredSchema`
  (`crates/db_core/src/ddl.rs::apply_core_schema_migrations`).
* Entity ids are **opaque strings, chosen by the caller**; there is no id
  generator in the DB. Conventions found:
  * jobs: UUID v4 string (`JobId(String)`);
  * tasks/comments: `new_id("task")` => `task-<nanos hex>-<counter hex>`
    (`crates/base/src/domain_support.rs`);
  * filestore: `file-cleanup-<uuid v4>`;
  * the workspace pins `uuid` features only for `v4` (jobs, app, file). UUID v7
    (time-sortable) would need a feature flag, and `Value::Uuid` is a distinct
    variant from the string id.
* There is **no entity versioning/history API** and no per-entity CAS
  (`docs/plans/2026-09-09-plugin-system/research-entity-versioning.md`).
  Concurrency control is a single storage revision.

### 1.3 Attribute ids and the qualified-key rule

Attribute and class ids are `:`-separated (dots also appear in legacy test
ids). Convention for shipped packages:

| Kind | Pattern | Examples |
|---|---|---|
| shared attribute | `semantic:<name>` | `semantic:title`, `semantic:created_at`, `semantic:parent` |
| class | `semantic:<domain>:<class>` | `semantic:jobs:job`, `semantic:tasks:task` |
| class-owned attribute | `<class id>:<name>` | `semantic:tasks:task:status` |
| relation class | `semantic:<domain>:<relation>` | `semantic:comments:entity_comment` |
| package name | dotted | `semantic.jobs`, `semantic.tasks` |
| module name | short | `v1` (jobs), `tasks`, `base`, `filestore` |

`crates/data/src/attr/mod.rs` defines the shared constants and the
`attr!`/`attrs!` marker macros:

```rust
pub const ATTR_TITLE: &str = "semantic:title";
pub const ATTR_DESCRIPTION: &str = "semantic:description";
pub const ATTR_CREATED_AT: &str = "semantic:created_at";
pub const ATTR_UPDATED_AT: &str = "semantic:updated_at";
pub const ATTR_PARENT: &str = "semantic:parent";          // Ref(any), shared hierarchy
pub const RELATION_CLASS_ID: &str = "semantic:relation";  // from / to / relation attrs
```

**Key rule (user decision, memory `project-qualified-attribute-keys`):**
entities that are class instances are keyed by *qualified attribute ids* in the
stored/returned `Object` (`{"id": "...", "type": "semantic:jobs:job",
"semantic:jobs:job:status": "queued", ...}`); queries default to
`FieldFormat::Qualified`. Plain names are accepted as decode aliases on
classes (`Marker::PLAIN_NAME` / field name), but writing both alias and
canonical is an error. Record types (RPC payloads, nested record attribute
values like job `progress`) use plain field names.

The class map inside `ClassType.attributes` is keyed by the *plain* name
(`"status"`) and points at the qualified attribute via
`ClassAttribute.attribute.id`.

### 1.4 Schema definitions (`crates/data/src/schema`)

Overview doc: `docs/schema.md` (hierarchy `SchemaDoc > modules > types /
attributes / classes / interfaces / contracts`, `Package > modules`).

**Type universe** (`schema/core/type_kind.rs`, `TypeKind`):
`Any Never Unknown Null Bool Char Number String Bytes Temporal Uuid IpAddr
Json | Optional Array List Tuple Map Set | Record Attribute Class | Union
Intersection Variant Enum | Result Function Interface Handle Stream | Opaque
Extension | Named(TypeRef) Ref(EntityRef)`. Only a *storable* subset can be
used for attributes: the catalog lowers each type to
`schema::lowered::DataType` and rejects unstorable attribute/record types in
`Catalog::apply_batch` with `CatalogError::UnstorableType` (see
`docs/schema.md` "Surface types vs. lowered data types"). Notably rejected:
`I256/U256`, `F80/F128`, `Decimal`, `BigInt`, `Period/Instant`, functions,
interfaces, handles, streams.

**Attribute** (`schema/attribute/attribute_type.rs`):

```rust
pub struct AttributeType { pub id: String, pub name: String,
    pub ty: Type, pub constraints: Vec<Constraint>, pub meta: Meta }
```

Attributes are **global and shared**: one attribute id can be referenced by
many classes (`semantic:title`, `semantic:parent` are). Owner module is
recorded by the migration's `module`.

**Class** (`schema/class/class_type.rs`, `class_attribute.rs`):

```rust
pub struct ClassType {
    pub id: String, pub name: String,
    pub inherits: Option<ClassRef>,        // single base
    pub extends: Vec<ClassRef>,            // mixins
    pub strict_schema: bool,               // key "semantic:class:strict_schema"
    pub creatable_in_ui: Option<bool>,     // "semantic:ui:creatable_in_ui"
    pub include_in_ui_listings: Option<bool>, // "semantic:ui:include_in_listings"
    pub attributes: BTreeMap<String, ClassAttribute>, // plain name -> config
    pub constraints: Vec<ClassConstraint>,
    pub meta: Meta,
}
pub struct ClassAttribute {
    pub attribute: AttributeRef, pub required: bool, pub ui_order: Option<u32>,
    pub computed: Option<Expr>, pub default: Option<Expr>,
    pub constraints: Vec<Constraint>, pub meta: Meta,
}
```

**Cardinality.** There is no explicit cardinality field. "At most one" is the
default (single value), "required" is `ClassAttribute.required`, "many" is a
`List`/`Set`/`Map` attribute type with `MinItems/MaxItems/Distinct`
constraints. Foreign keys are a `Ref` type.

**References/relations.**

* `TypeKind::Ref(EntityRef { target: Option<TypeName>, on_delete })` stores the
  primary id of an entity (`schema/core/entity_ref.rs`):
  `OnDelete::Restrict` (default) rejects deleting a still-referenced target;
  `OnDelete::Cascade` deletes the owner transitively and atomically.
  `target: None` (`EntityRef::any()`) accepts any class. Ref targets are
  *classes* and subclasses are accepted. `docs/validation.md` details
  enforcement.
* `RelationType` (`schema/relation/relation_type.rs`) models **edge entities**:
  a class inheriting `semantic:relation` (attrs `relation`, `from`, `to`)
  plus a `RelationType { id, name, source_collection, mode: External |
  Embedded{attribute}, indexing_mode }`. Used by labels
  (`semantic:base:entity_label`), directories and comments
  (`semantic:comments:entity_comment`). Caveat documented in
  `crates/base/src/labels/mod.rs`: generic indexed relation predicates use
  *bare* endpoint ids and cannot distinguish same-id entities in different
  collections; comments add `entity_collection` to disambiguate.

**Enums, variants, records.**

* `EnumType { repr: EnumRepr::String|..., variants: Vec<EnumVariant> }` is how
  string enums such as task status are persisted
  (`crates/base/src/tasks/migration_v1.rs::string_enum`).
* `VariantType { tag: VariantTag, variants: Vec<VariantCase> }` with tags
  `ExternallyTagged | InternallyTagged{field} | AdjacentlyTagged{tag_field,
  data_field} | Untagged`; payload `Unit | Tuple | Record | Newtype`. The
  `#[semantic(tag = "kind")]` enum derive produces an **InternallyTagged**
  variant; `MainContent` (`crates/base/src/content/schema.rs`) hand-builds an
  **AdjacentlyTagged** `{kind, data}` variant whose payload is a full embedded
  `TypeKind::Class`.
* `RecordType { fields: BTreeMap<String, Field>, open, additional, ... }` for
  nested structured values (job `progress`/`error`).

**Constraints** (`schema/constraints/constraint.rs`): `Min Max MultipleOf
Length Pattern Prefix Suffix Contains Precision Charset Collation TimeZone
MinItems MaxItems MinProperties MaxProperties RequiredFields KeyPattern
Unique Distinct PrimaryKey Index DefaultValue DefaultExpr Transport`
(+ hidden legacy foreign key). Per `docs/validation.md`, only `Min Max Length
Pattern Prefix Suffix MinItems MaxItems MinProperties MaxProperties
RequiredFields` are *enforced*, and recursive validation (required attrs,
nested records, refs) only after an explicit per-DB
`activate_validation()`. Until then writes in a `StrictRegisteredSchema`
collection still normalize objects and reject unknown fields /
top-level type mismatches (`ObjectNormalizationError::{UnknownField,
TypeMismatch}` in `crates/db_core/src/validation.rs`), but domain services
must validate their own invariants (the tasks implementation notes say the
same: "Existing low-level database writes do not enforce all attribute
constraints").

**Indexes** (`schema/index`, `MigrationDdlOperation::UpsertIndex`):
`IndexKind::{Equality, PathEquality, Range, FullText}`, `unique`, composite via
`extra_fields`, partial via `predicate`, full-text `analyzer`. Index `field` is
the *qualified attribute id* (jobs: `format!("{PREFIX}{field}")`).

**Collections** (`MigrationDdlOperation::UpsertCollection { name, kind:
Untyped|Schema|Polymorphic, integrity_mode: Permissive|StrictRegisteredSchema
}`): `Polymorphic` + `StrictRegisteredSchema` is what jobs use; `Polymorphic`
is normalized to `Schema` by the catalog.

**Metadata/UI hints.** `Meta { title, description, aliases, tags,
annotations, ... }`. `ClassType.creatable_in_ui` and `include_in_ui_listings`
(`false` excludes) control the generic UI. Hide anything that is not a
user-facing entity (events, turns) with both set to `Some(false)` (comments set
`include_in_ui_listings: Some(false)`; tasks, comments and jobs set
`creatable_in_ui: Some(false)`).

### 1.5 Core schema

* Core attributes/helpers: `crates/data/src/attr/mod.rs` (title, description,
  created_at, updated_at, url, parent, relation from/to/relation, UI flags).
* Core migrations (catalog entry classes, `semantic:url` + index, UI flags,
  shared attributes, reference lifecycle, index definitions, full-text,
  registration proofs, timestamp indexes, include_in_listings) live in
  `crates/db_core/src/ddl.rs::core_schema_migrations()` (011 migrations at time
  of writing) and are applied by `apply_core_schema_migrations`.
* Shared attributes owned by the cross-package `shared` module:
  `crates/data/src/bundles/shared.rs` (`semantic:title`, `semantic:description`,
  `semantic:parent`). **Reuse these** for agent entities (title, parent for
  forks) instead of redefining.
* Other packages in `crates/data`: `filestore` (file class + object-store
  locators + cleanup intents), `jobs`, `plugin` (activation records), `import`
  (interfaces `Source/Fetcher/Importer`), `bundles/query` (query AST as
  `TypeDef`s with a frozen `v1-definitions.json` snapshot), `bundles/auth`
  (`semantic:auth:user`, collection `_semantic.auth`), `bundles/directory`.

---

## 2. `crates/macros`: derive macros

`crates/macros/src/lib.rs` (+ `model.rs`), re-exported from
`semantic_data::value` and `semantic_data` root:

| Derive | Generates |
|---|---|
| `SemanticType` | `impl SemanticType { fn semantic_type() -> Type }` |
| `IntoValue` | `impl IntoValue { fn into_value(self) -> Value }` |
| `FromValue` | `impl FromValue { fn from_value(Value) -> Result<Self, FromValueError> }` |
| `Class` | all three **plus** `impl ClassDescriptorConst { const ID }` |

The macro-generated code uses absolute `::semantic_data::...` paths, so a
downstream crate must depend on `semantic_data` by that name (the crate
itself has `extern crate self as semantic_data;`).

Supported shapes:

* struct with named fields => **record** (object keyed by plain field names);
* newtype struct => transparent (`JobId(String)`);
* enum of unit variants => string enum (`#[semantic(rename_all =
  "snake_case")]`) => `TypeKind::Enum` with `EnumRepr::String`;
* enum with `#[semantic(tag = "field")]` => object with the tag in `field` plus
  struct-variant fields => `VariantType` InternallyTagged;
* `Class`: struct + `#[semantic(id = "...")]` [+ `namespace = "..."`].

Attributes: container `rename_all`, `tag`, `id`, `namespace`; field `rename`,
`default`, `default = "path::fn"`, `flatten`, `required` (on `Option`: always
encoded, `None` as null), `attr = Marker` (class fields only); variant
`rename`. `Option<T>` is optional (missing/null decode to `None`, `None` is
omitted on encode). Doc comments become field/variant descriptions.

Class field keys: `<namespace or class id>:<field name>` unless `attr =
Marker` pins an explicit shared id (`#[semantic(attr = AttrId)]` for the
builtin `id`; `AttrCreatedAt`, `AttrTitle`, `AttrParent` in
`crates/data/src/attr/mod.rs`). Encoding writes `type = <class id>`; decoding
accepts a missing `type` but rejects a different one.

Marker declaration:

```rust
semantic_data::attrs! {
    /// A score.
    Score, "test:scoring:score", u32;
    Label, "test:label:name", String, name = "label";
}
// optional: , schema = expr   to supply the full AttributeType (e.g. a Ref)
```

`AttrDescriptor::attr_schema()` returns an `AttributeType` (default built from
the value's `SemanticType`; override with `schema =`). This is how
`AttrParent` ends up as `Ref(any)` although its Rust type is `String`.

`SemanticType` impls exist for: `Value, (), Null, bool, String, i8/i16/i32/
i64, u8/u16/u32/u64/usize, f64, DateTime, Option<T>, Vec<T>, BTreeMap<String,
T>, Object, Box<T>, StreamOf<T, End>`. **Not implemented** (so wrap or
hand-implement, as tasks' `DueDate` wraps `Date`): `Date`, `Time`, `Duration`,
`Uuid`, `Bytes`, `i128/u128`, `f32`, `BTreeMap<K != String, _>`, `HashMap`. Hand-rolled impls are the established workaround
(`crates/base/src/tasks/model.rs::DueDate`, `content::MainContent`).

Usages in the codebase:

* `crates/data/src/jobs.rs`: `JobRecord` is `#[derive(facet::Facet, Class)]
  #[semantic(id = "semantic:jobs:job")]` with `#[facet(rename = ...)]`
  mirroring the qualified keys for the SDK export; enums/records/newtypes use
  `SemanticType, IntoValue, FromValue`.
* `crates/base/src/server.rs`: `ServerConfig` (communication-only class, not
  registered in a schema).
* `crates/data/tests/derive.rs`: exhaustive behavior tests.
* RPC payloads everywhere: `#[derive(SemanticType, IntoValue, FromValue)]`
  structs (`crates/base/src/tasks/commands.rs`, `crates/app/src/jobs/commands.rs`).
* Tasks/comments/labels do **not** use `#[derive(Class)]`; they hand-write
  `from_object`/`to_object` (older style, because they preserve unknown
  attributes when patching). New code should prefer the derive.

`facet::Facet` is a second, independent derive used for (de)serialising
schema documents/`Package` as JSON (`facet_json`) and for `semantic_sdk_export`
(TypeScript generation, `crates/sdk_export/src/main.rs`: `with_class::<T:
Facet + ClassDescriptorConst>`). Add `#[derive(facet::Facet)]` to
classes/payloads that the web SDK must see.

### 2.1 Gap: how a Rust type "declares its schema"

Today a Rust type declares its **value type** (`SemanticType`), not its
registered **class/attribute definitions**. The registered definitions are
hand-written `ClassType`/`AttributeType` values in the package, and a test
proves they agree. The jobs test shows the pattern
(`crates/data/src/jobs.rs`, `rpc_query_defaults_and_types_match_the_contract`):

```rust
let TypeKind::Record(record) = JobRecord::semantic_type().kind else { panic!() };
let class = &package().root.classes[CLASS_ID];
for attribute in class.attributes.values() {
    assert_eq!(record.fields[&attribute.attribute.id].required, attribute.required);
}
```

If "types defined as semantic schema classes" is meant to be *generated* from
the Rust structs, a new trait (e.g. `ClassSchema { fn class() -> ClassType;
fn attributes() -> Vec<AttributeType> }`) and derive support would have to be
added to `crates/data` + `crates/macros` (data-model-first layering, memory
`feedback-data-model-first-layering`). That is a core change. Decision for the
user; see section 11. Regardless of the choice, **migrations must never call
the live schema generator**: they carry frozen snapshots.

---

## 3. Migrations and packages

### 3.1 Structures

`crates/data/src/schema/migration/mod.rs`:

```rust
pub struct Migration { pub module: String, pub name: String,
    pub description: Option<String>, pub operations: Vec<MigrationOperation>, pub meta: Meta }
pub enum MigrationOperation { Ddl(MigrationDdlOperation),
    Insert{collection,id,object}, Update{query: UpdateQuery}, Delete{query: DeleteQuery} }
pub enum MigrationDdlOperation {
    UpsertAttribute{attribute}, DeleteAttribute{id},
    UpsertTypeDef{..}, DeleteTypeDef{..}, UpsertRecordType{..}, DeleteRecordType{..},
    UpsertClass{class}, DeleteClass{id},
    UpsertCollection{name, kind, integrity_mode}, DeleteCollection{name},
    UpsertIndex{name, collection, field, unique, kind, extra_fields, predicate, analyzer},
    DeleteIndex{..}, UpsertRelationship{relationship}, DeleteRelationship{id}, SetAutoIndex{..} }
```

`crates/data/src/schema/package/package.rs` and `module/module.rs`:

```rust
pub struct Package { pub name: String, pub root: Module,
    pub modules: BTreeMap<String, Module>, pub migrations: Vec<Migration>,
    pub version: Option<SchemaVersion>, pub meta: Meta }
pub struct Module { pub name: String, pub constants, pub types, pub attributes,
    pub classes, pub interfaces, pub contracts, pub meta: Meta }
```

Migrations apply per `module`; the migration key is
`package::module::name` (`applied_migration_key` in
`crates/db_core/src/managed_schema.rs`). Data operations (`Insert`, `Update`,
`Delete`) are interleaved with DDL (DDL is batched until a data op).

### 3.2 Semantics enforced by the DB

* `Db::upsert_package(package)` -> `PackageRegistrationOutcome {
  executed_migrations }` (`crates/db_core/src/backend.rs`,
  `crates/db_core/src/embedded/db.rs::apply_package_update`).
* For each migration in order: if already applied *and the stored migration
  `!=` the presented one* => error
  `applied migration '<module>::<name>' for package '<pkg>' differs from the
  stored definition` (policy `Fail`, or logged with `Log`:
  `crates/db_core/src/config.rs`). Else apply and record it.
* `validate_package_migrations(&package)` (`managed_schema.rs`) replays the
  migrations on a fresh core catalog and **requires each module's declared
  `types/attributes/classes` to equal the replay result**. Hence the package
  `root` module is always the *current* snapshot and must be updated in
  lock-step with each new migration.
* Unchanged re-registration after reopen does zero storage work (certificate
  fast path; test `unchanged_filestore_registration_after_reopen_does_no_storage_work`).
* Foreign (cross-package) classes referenced by a package must be installed
  first. Optional packages also declare a **frozen dependency module** that
  re-declares the foreign attributes they use (needed by replay validation;
  see `crates/base/src/domain_support.rs::with_content_dependency` inserting a
  `base` module + migration `001_embedded_content_dependency` ahead of the
  tasks migration). `validate_package_migrations_with_catalog` takes the
  installed catalog as dependency context.
* Catalog loading never rejects existing DBs; batches reject unstorable types.

### 3.3 Rules from AGENTS.md (verbatim intent)

> Entities and schema definitions in core (`crates/data`) and the base package
> (`crates/base`) must always be introduced or changed through migrations.
> Never modify an existing migration: its definition is persisted in user
> databases and must remain valid and byte-for-byte equivalent in meaning. Add
> a new forward migration for every schema change, and keep historical
> migration helpers and snapshots isolated from current schema definitions.

How the repo enforces it:

1. **Frozen helpers.** `crates/base/src/migration_support_v1.rs` ("Frozen
   construction helpers for content/tasks/comments initial migrations") and
   `crates/base/src/tasks/migration_v1.rs` re-declare attributes/classes
   instead of calling `tasks::schema` (which is the *current* definition and
   may change). Old migrations of `notes_v1` / `notes_v2` coexist with the
   current `notes::class()`. `bundles/query` embeds a JSON snapshot
   (`v1-definitions.json`).
2. **Hash pinning.** `crates/base/tests/listing_migrations.rs` +
   `listing_migrations.sha256` hash `facet_json::to_string(&migration)` of every
   historical migration of core/base/files/jobs/plugins/comments; newly added
   migrations without a recorded hash are skipped, so adding a forward
   migration is allowed but silently editing an old one fails the test.
   (Tasks is not yet in that list; a new agent package should be added.)
   `package_registration.rs` additionally pins nine historical migrations by
   SHA-256.
3. **Forward-compat patterns.** `ClassType`/`EntityRef`/`IndexSchema` use
   `#[facet(default)]`, `skip_serializing_if` and legacy aliases so old stored
   migration JSON still decodes to an equal value. New optional fields on
   core types must follow the same pattern (and `migration` equality is
   structural on the decoded value).
4. **Upgrade tests.** `packages_upgrade_from_before_listing_metadata`: install
   the previous version of a package, then the current, assert exactly the new
   migrations execute.

### 3.4 Is there a notion of schema packages outside `base`? Yes.

| Package | Where defined | Installed by |
|---|---|---|
| `semantic.base` | `crates/base/src/bundle`, `migrations` | app default (`default_packages`, `BasePackage`) |
| `semantic.filestore` | `crates/data/src/filestore.rs` | app default |
| `semantic.query` | `crates/data/src/bundles/query` | app builder `build()` |
| `semantic.import` | `crates/data/src/import.rs` | app builder `build()` |
| `semantic.jobs` | `crates/data/src/jobs.rs` | `DbJobStore::initialize` (`crates/app/src/jobs/store.rs`) upserts it |
| `semantic.plugin` | `crates/data/src/plugin.rs` | plugin activation store |
| `semantic.comments`, `semantic.tasks` | `crates/base/src/comments`, `tasks` | app builder if enabled (`register_package(CommentsPackage/TasksPackage)`) |
| `semantic.vdb` (planned) | `crates/data/src/vdb.rs` (docs/plans/2026-10-03-plugin-dbs/plan.md T1.1) | app |

Plugin-system design (`docs/plans/2026-09-09-plugin-system/design.md` section
4.1): "Use built-in `semantic.plugin` and `semantic.import` packages, named
`v1` module interfaces, and ordinary package migrations for persisted
classes/attributes. Jobs owns its separate package." and section 4.2:
"Package registration and process startup are separate: installed schema may
remain after startup fails." Plugins do **not** ship their own persisted
schema today: plugin-provided data is exposed as a *runtime-only* schema
overlay (plugin-dbs plan: "Runtime-only, explicitly exposed definitions; no
migrations, nothing persisted"). So there is no precedent for third-party
plugins shipping migrations; first-party Rust packages do.

Package installation scoping (from `crates/app/src/command.rs` and
`docs/plans/2026-09-30-task-system/implementation.md`):

* `SemanticAppBuilder::register_package(impl RuntimePackage<Ctx, E>)`
  registers commands app-wide and queues `package.schema()`; schemas are
  applied **lazily to the default DB before first use**
  (`initialize_default_db`).
* Explicitly opened scopes **retain their existing schema initialization**;
  packages must be installed there explicitly (RPC
  `semantic.db.package.upsert` with a facet-json `Package`, or code).
* Feature toggles: `AppConfig::with_tasks(bool)`, env `SEMANTIC_TASKS`;
  `semantic.app.capabilities` (`crates/app/src/capabilities.rs`) reports a
  feature available only when **both** the command is registered **and** the
  scope catalog contains the class. An `agents` feature should follow the same
  shape.
* Disabling a feature never removes persisted schema/data.

### 3.5 Where should the agent schema live?

Recommendation: **its own package `semantic.agent`** (module `v1`, own
migration list starting `001_...`) defined in the new reusable data-model
crate (portable, depends only on `semantic_data`; compiles to wasm so the UI
can use the types). Reasons: independent upgrade cadence, zero edits to
core/base migrations, same pattern as jobs/tasks/vdb, installable per scope.
Constraints:

* Depend on `shared` attributes (`semantic:title`, `semantic:parent`,
  `semantic:created_at/updated_at`) by id; declare the `shared` module/
  migration in the package like filestore does
  (`crates/data/src/filestore.rs::package()` includes
  `bundles::shared::module()` + `migration_v1()`), so install order of
  filestore/base/agent never matters.
* If threads reference tasks/files/notes via `Ref(target = class)`, the
  replay validator needs those classes (see `with_content_dependency`). To
  avoid hard install-order coupling use `EntityRef::any()` for `subject` (as
  `semantic:parent` does) or a relation entity (section 10.5).

---

## 4. `crates/base`: built-in schema package inventory

`crates/base/src/bundle/mod.rs` builds `semantic.base`, module `base`
(plus `shared`). Classes in `base`:

| Class id | File | Notes |
|---|---|---|
| `semantic:base:person` | `schema/common/person.rs` | people |
| `semantic:base:note` | `schema/notes.rs` | `note_format` (`text`/`markdown`), `note_content`; forward migrations `notes_v1`, `notes_v2` (markdown default) |
| `semantic:base:web_bookmark` | `schema/web_bookmark.rs` | uses core `semantic:url` |
| `semantic:base:directory`, `directory_node` (+ relation `semantic:base:directory_node`) | `crates/data/src/bundles/directory.rs` | folders |
| `semantic:base:label`, `label_group` (+ relation `semantic:base:entity_label`) | `schema/labels.rs`, `labels/` | hierarchical labels, membership relation |
| attribute `semantic:base:main_content` | `content/` | tagged embedded content variant (`kind: note`) |

Base package migration list (13 at time of writing): `001_init` ... `013_include_in_listings`
(`crates/base/src/migrations/mod.rs::all()`).

Optional packages in the same crate:

* `semantic.comments` (module `comments`): `semantic:comments:comment`
  (`main_content`, `author_id`, `semantic:parent` for replies, deleted
  tombstone, timestamps), relation class `semantic:comments:entity_comment`
  (from = subject id, to = comment id, `entity_collection`), migrations
  `001_comments`, `002_include_in_listings`.
* `semantic.tasks` (module `tasks`): `semantic:tasks:task`.

There is **no "project", "file notes", "agent" or "session" class** in base.
Files: `semantic:filestore:file` (+ `semantic:filestore:cleanup`) in the
filestore package; bytes live in an object store (`objstore`) addressed by
`filestore_locator`; metadata attributes include `byte_size`, `mime_type`,
`content_hash_sha256`, `filekind`, media attrs. Jobs: `semantic:jobs:job`.
People: `person` (the comment `author_id` is the request principal id string,
not a Person ref).

---

## 5. Task system and how agents could relate

Docs: `docs/plans/2026-09-30-task-system/{plan,implementation,research}.md`.
Code: `crates/base/src/tasks/*`, `crates/app/src/task_comments.rs`,
`crates/ui_core/src/ui_catalog/tasks_comments.rs`.

**Task model** (`tasks/schema.rs`, class `semantic:tasks:task`, stored in the
default collection `entities`, `creatable_in_ui: Some(false)`, not strict):
`title` (`semantic:title`), `main_content` (`semantic:base:main_content`,
embedded Note variant), `status` (enum `backlog todo in_progress blocked done
canceled`), `priority` (`none low medium high urgent`), `progress` (u64 <= 100),
optional `due_date` (`Date`), optional `parent` (`semantic:parent`, a task),
`archived` bool, `created_at`/`updated_at`. Hierarchy uses the shared parent
attribute; labels use the existing `entity_label` relation; comments use
`entity_comment`. Deliberately left for later migrations: assignees, projects,
cycles, dependencies, named workflows.

**Service pattern** (copy this): a `*Context` trait in the domain crate
(`TaskContext { type Store: LabelStore; fn task_store(&self, scope_id) -> ... }`)
implemented for `AppRequestContext` in the app crate
(`crates/app/src/task_comments.rs`), so the domain crate depends only on
`semantic_data` + `semantic_rpc_core`. `LabelStore` is the minimal DB seam
(`select(sql) / get(collection,id) / commit(Batch)`), writes are serialized by
a process-wide `futures::lock::Mutex` and committed as one `Batch`.
Commands are `RpcCommand`s grouped into a `RuntimePackage { schema(),
commands() }`.

**How agent threads/runs could relate to tasks**

1. *Thread -> task link (recommended):* optional `subject` attribute on the
   thread (`Ref` to any class, like `semantic:parent`) or an
   `entity_agent_thread` relation modeled on `entity_comment` (collection-
   qualified, indexed, many threads per task and vice versa). A task detail
   page can then list "Agent runs" like it lists comments, via the same
   generic embedding mechanism.
2. *Task as prompt source:* a turn's input can be seeded from a task's
   `main_content` (`MainContent::body()`); store the resulting text on the
   turn (snapshot) rather than a live reference, so edits do not rewrite
   history.
3. *Outcome back-links:* write agent results back as **comments** on the task
   (generic `semantic.comments` API, `author_id` = principal id of the agent
   runner) rather than inventing a parallel message model for the task view.
4. *Status automation:* optional, behind an explicit feature (tasks status is a
   validated command surface; `TaskStatus::InProgress/Done` with the
   done=>100% progress rule).
5. Do **not** make the agent crate depend on `semantic_base::tasks`; keep the
   link by id + class string so the agent package installs without tasks.

---

## 6. Jobs system and agent runs

Docs: `docs/plans/2026-09-10-jobs-system/design.md`; code `crates/jobs`,
`crates/data/src/jobs.rs`, `crates/app/src/jobs/*`.

Contract (design section 1, binding):

* One coordinator per scope, executes several jobs concurrently up to
  `JobsConfig { max_concurrent_jobs: 4 (default), history_threshold: 1000 }`
  (`SEMANTIC_JOBS_CONCURRENCY`).
* Persisted: only `JobRecord` (kind, status Queued/Running/Cancelling/
  Succeeded/Failed/Cancelled/Interrupted, `JobProgress{completed,total,unit,
  phase}`, `JobError{code,message}`, timestamps, `snapshot_seq`) in collection
  `semantic_jobs`, polymorphic strict. Progress writes are coalesced (<= 1/s
  per active job).
* **Runtime-only:** inputs, outputs, checkpoints, stream contents, handler
  state. "A restart cannot reconstruct unfinished work. Previously nonterminal
  jobs become `Interrupted`; ... There is no implicit retry, resume, or replay."
* API: `JobHandler { type Input: Send; type Output: Send; kind(); run(input,
  JobContext) }`, `JobContext { id, cancellation: CancellationToken,
  report_progress }`, `ScopeJobs::submit` returning a `JobTicket<T>` (`wait`,
  `subscribe() -> watch::Receiver<JobRecord>`), `JobGroup` cancellation
  groups (plugin generation invalidation), commands `semantic.jobs.{list,get,
  cancel,clear_completed,kinds}` (polling; a later `watch` over interface
  streams is planned but not built).
* App integration: `ScopeManager::resolve_jobs` creates exactly one
  `ScopeJobs` per scope and *pins the DB* against idle retirement
  (`retire_idle_scopes` skips scopes with jobs); `close_scope_with_jobs` shuts
  down asynchronously. Plugins get the same treatment (`state.plugins`).

Could agent runs be built on it? Assessment:

| Need | Jobs fit |
|---|---|
| Cancellation token, panic isolation, typed async handler | Good |
| Interrupted-on-restart reconciliation | Good pattern (copy) |
| Visibility in an existing "Jobs" UI, cancel button | Good if we mirror a record |
| Long-lived (hours), interactive, bidirectional session with approvals and follow-up input | **Poor**: a handler takes one input and returns one output; no mid-run input channel besides handler-private channels |
| Resume after restart (CLI session id + persisted transcript) | **Contradicts** "no resume/replay"; resume must be orchestration-owned |
| Concurrency | **Poor**: global limit 4 (default) is shared with imports/media; N agent sessions would starve it, and a high limit defeats its purpose |
| Streaming progress (tokens/tool events) | **Poor**: `JobProgress` is `completed/total/unit/phase`; no event stream |
| History retention | **Conflict**: `history_threshold` deletes oldest terminal job records; agent transcripts are user data, not operational history |
| Per-scope DB pin / shutdown hooks | Reusable idea; hard-wired to jobs + plugins in `ScopeManager` |

Recommendation: the orchestration crate owns an **AgentSupervisor** (tokio
tasks, process handles, per-thread command channels, `CancellationToken`s)
and its own persisted state. Optional later: register a thin `semantic.agent.turn`
job kind purely for observability if the concurrency cap is made per-kind or
excludable (needs jobs-system design input; not a blocker). A required new
app seam is a generic "scope service" lifecycle (pin DB while active, orderly
shutdown) since `ScopeManager` special-cases jobs and plugins (section 11).

---

## 7. `crates/db_core`: transactions, queries, change feed, performance

### 7.1 Facades

* `semantic_db_core::Db` (wrapper over `Backend`, `crates/db_core/src/backend.rs`):
  `catalog`, `begin_transaction`, `subscribe_changes`, `scan_entities`,
  `create_collection`, `execute_ddl`, `upsert_package`, `insert/get/delete`,
  `query(QueryInput)`, `execute_batch*`, maintenance (`verify`, `reindex`,
  `backup`, `compact_storage`, `export_snapshot`, ...).
* `semantic_app::SemanticDb` (`crates/app/src/db.rs`) is the trait services use
  (impl for `Db`, mocks in tests): `catalog, query(TextQueryInput),
  query_data, get, insert, delete, execute_batch*, execute_batch_returning*,
  upsert_package, parse_sql, validation_*, maintenance`. It does **not**
  expose `begin_transaction` or `subscribe_changes`.

### 7.2 Writes

`semantic_data::query::BatchOperation` (`crates/data/src/query/mod.rs`):

```rust
Create{collection,id,object}   // fails with EntityExists if present
Upsert{collection,id,object}   // whole-row replace
DeleteById{..} DeleteByIds{..}
Update{collection, query: UpdateQuery}  // predicate + assignments (SET ...), limit
Delete{collection, query: DeleteQuery}
```

* A `Batch` is atomic. Use `execute_batch_returning(batch,
  BatchReturn::Stats)` (or `Changes` / `Projection`) for appends: the default
  `execute_batch` returns the touched rows (`Dataset`) and was O(rows
  returned); after commit `perf(db): cut write latency...` it is bounded by
  the batch, but `Stats` is the cheapest.
* Interactive transactions (`TransactionHandle`: `get/select/upsert/create/
  delete/update_where/delete_where/execute_batch/savepoint/rollback_to/
  commit`) exist on `Backend` for the embedded engines only; isolation levels
  `ReadCommitted | RepeatableRead | Snapshot | Serializable`; commit-time
  conflict => `DbError::TransactionConflict` (retry policy `ConflictPolicy`).
  **Postgres: default `Unsupported`.** Not reachable through `SemanticDb`.
* Conflict detection is **whole-DB revision**, not per entity. Concurrent
  non-conflicting writers are retried by the engine
  (`TransactionOptions::max_retries = 3` default).
* Existing services (tasks/labels/comments) avoid read-modify-write races with
  an in-process async mutex plus a single batch. The orchestrator is a
  single in-process owner of its threads, so a per-thread mutex is enough.

### 7.3 Reads

* SQL (default dialect generic; `FORMAT QUALIFIED`/`PLAIN`/`UNDERSCORE` select
  attribute-key spelling; default `Qualified`), PRQL, or the typed query AST
  (`QueryInput::Ast`, params). Services in base build SQL strings with
  `sql_ident`/`sql_string` helpers
  (`crates/base/src/directory_query.rs`), e.g.
  `SELECT * FROM "entities" WHERE type = 'semantic:base:label' FORMAT QUALIFIED`.
  Prefer the AST/params for anything with user data.
* Point `get(collection, id)` ~10 us (memory/redb; `docs/benchmarks.md`).
* Index-backed: equality (`eq_select` ~50 us), ordered range scans with LIMIT
  (`ordered_index_scan` ~240 us), **keyset pagination** (`keyset_page`
  ~240 us), composite `(kind, created_at)` feed query (~130 us), full text
  (`text_match(col..., 'a b')`, ~1 ms).
* Class queries are **exact `type = '...'`**; there is no subclass-aware
  predicate (no `instance_of`). So polymorphic event hierarchies should not
  rely on `inherits` for querying; use one class with a discriminant, or a
  dedicated collection.
* Readers use snapshots outside the writer lock (`concurrent_read` ~165-390
  us with a concurrent writer), so UI reads do not stall on agent writes.

### 7.4 Change feed / subscriptions

`crates/db_core/src/change_feed.rs`:

* `Backend::subscribe_changes(ChangeSubscriptionOptions { include_payloads,
  include_internal, collections }) -> ChangeStream` yields
  `ChangeFeedItem::Event(ChangeEvent { revision, committed_at, source:
  Batch|Transaction|Ddl|Migration|Maintenance, changes: Vec<ChangedEntity
  {collection,id,kind: Created|Updated|Deleted,before,after}>,
  catalog_version, catalog_changed })` or `Lagged{skipped, resume_revision}`.
* Published while holding the writer lock => events are in commit order;
  failed/conflicted commits publish nothing; bounded per-subscriber buffer
  (`DEFAULT_CHANGE_FEED_CAPACITY = 1024`) with lag notices; **no history
  replay** ("clients that need a consistent view read the current state and
  then apply events with a higher revision").
* Filter granularity is the **collection**, not the class; hence a dedicated
  collection for high-volume events lets a subscriber ignore everything else
  (and vice versa).
* Availability: `EmbeddedBackend` only (kv/redb/log engines). Postgres and the
  federation wrapper don't forward it. No RPC/`SemanticDb`/UI exposure today
  (grep confirmed: only `db_core`, `db_test`, `db_redb` tests use it).
* UI today refreshes by polling (`semantic.jobs.list`), as the jobs design
  states ("polling already supports the initial view").

### 7.5 Streaming to clients (the missing live-update path)

* `crates/rpc/src/stream_command.rs`: `RpcStreamCommandSpec { Payload, Input:
  InputShape (() | StreamOf<T,End>), Output: OutputShape (Single<T> |
  StreamOf<T,End>) }`, `RpcStreamCommand::call(ctx, payload, input, cancel:
  CancellationToken) -> BoxFuture<Result<TypedStream<..>, Error>>`,
  `TypedStream::from_events(stream of TypedEvent::{Item, End})`.
* Registered with `SemanticAppBuilder::register_stream_command`, served over
  interface sessions (export `semantic.command`); client side
  `RpcClient::invoke_stream::<C>(payload, input)`. Implementations of
  `invoke_interface` exist for the wasm HTTP client
  (`rpc/src/transport/http_client_wasm.rs`), native HTTP client, and the
  embedded UI backend (`crates/ui/src/backend.rs`), and the server tests cover
  WebSocket sessions (`server/src/lib.rs`). No production command uses
  streaming yet; the only users are test commands (`test.streaming.echo`,
  `StreamCount`...). Treat as ready-but-unproven for the UI.
* Therefore the realistic live design is: **orchestrator publishes committed
  events on an in-process broadcast (tokio `broadcast`/`watch`) after the DB
  commit, and an `agent.thread.subscribe` stream command bridges that to the
  client**, with `after_seq` catch-up from the DB (query `seq > ?`), de-dup by
  `(thread, seq)`. This reproduces the change-feed contract (state + tail) in
  the orchestration layer and works on every backend.

### 7.6 Performance for high-frequency appends

Measured (`docs/benchmarks.md`, 10k rows, AMD 5800X NVMe; indicative):

| Operation | memory | redb (Eventual durability in bench) |
|---|---:|---:|
| insert batch 100 (`execute_batch`) | 12 ms (~8k rows/s) | 31 ms (~3.3k rows/s) |
| insert batch 1000 | 134 ms | 198 ms (~5k rows/s) |
| insert with fsync every commit (batch 100) | n/a | 30 ms (~3.3k rows/s) |
| `update_by_id` | 0.16 ms | 1.55 ms |
| `delete_by_id` | 0.13 ms | 2.5 ms |

* ~130-190 us per inserted row with the benchmark's 12 attributes and 4
  indexes; each additional index on the row adds work; `Ref` attributes add
  reverse-reference maintenance once validation is activated
  (`docs/validation.md`; reverse refs are system-collection rows).
* **redb default durability is `Immediate` (fsync per commit)**
  (`crates/db_redb/src/options.rs`). Per-token commits are therefore a
  non-starter; commit rate per active run must be bounded (see 10.4).
* One writer at a time (writes are prepared lock-free on a snapshot, then
  committed under the writer lock with conflict retry). N concurrent agent
  runs contend on one commit queue; keep each commit small and infrequent.
* `Upsert` re-encodes the whole row (compact v2 payloads with a field
  dictionary: `crates/db_kv/src/storage/entity_codec.rs`). Appending to a
  growing text field by repeated upsert is O(n^2) bytes written.
* No documented maximum entity/string size was found; only `MAX_VALUE_DEPTH =
  128` nesting. Treat rows as small (KB, not MB).
* Large text/blobs: use the filestore (`semantic:filestore:file` + object
  store, create-only, hash/UUID locators; `docs/file-maintenance.md`) and
  reference by id. `crates/media` is analysis (MIME sniffing/ffprobe), not
  storage.

---

## 8. Runtime integration pattern (domain crate to app)

Pattern used by tasks/comments/labels and jobs:

1. **Domain crate** (portable) exposes `package() -> Package`, DTOs
   (`SemanticType + IntoValue + FromValue` structs), services generic over a
   store trait, commands as `RpcCommandSpec + RpcCommand<Ctx>`, and
   `struct XPackage; impl<Ctx: XContext, E: From<RpcError>> RuntimePackage<Ctx,
   E> for XPackage { schema(), commands() }`
   (`crates/base/src/tasks/mod.rs`, `rpc_core/src/package.rs`).
2. **App crate** implements `XContext for AppRequestContext` (scope +
   authorization source: `ctx.principal`, `ctx.resolve_db(scope_id)`), adds a
   builder flag + `AppConfig` field + env var, and calls
   `register_package(XPackage)` in `SemanticAppBuilder::build()` behind the
   flag (`crates/app/src/command.rs`).
3. **Jobs-style runtimes** (long-lived per-scope services) are created via
   `ScopeManager::resolve_jobs` and tracked in `ScopeState` so scope close and
   idle retirement account for them. There is no generic registration hook;
   an agent supervisor requires adding one or special-casing like jobs.
4. **Capabilities + UI gating:** extend `semantic.app.capabilities` with an
   `agents: bool` (command registered AND class in catalog), as for tasks.
5. **UI:** reusable components in `crates/ui_core`
   (`ui_catalog/tasks_comments.rs` registers class renderers; generic
   renderers/forms keyed by class id), routes in `crates/ui`.
6. **SDK:** add the package to `crates/sdk_export/src/main.rs` and Facet roots
   so TypeScript types are emitted.

---

## 9. End-to-end recipe: add a new domain schema

Concrete steps, using a hypothetical `semantic.agent` package. File names are
suggestions that mirror `crates/data/src/jobs.rs` and `crates/base/src/tasks`.

### Step 1 - crate and dependencies

`crates/agent_core/Cargo.toml` (workspace members are `crates/*`, so no
workspace edit; see `crates/jobs/Cargo.toml`):

```toml
[package]
name = "semantic_agent_core"
version.workspace = true
edition.workspace = true
license.workspace = true

[dependencies]
semantic_data.workspace = true   # also brings the derives (semantic_data::{Class,...})
facet = { version = "0.43.2" }   # only if SDK export / package JSON needs Facet
```

Keep it free of tokio/process deps so the UI (wasm) can import the types.

### Step 2 - ids, enums, records, payloads (derives)

```rust
use semantic_data::{Class, FromValue, IntoValue, SemanticType, DateTime};

#[derive(SemanticType, IntoValue, FromValue, Clone, Debug, PartialEq, Eq, Hash)]
pub struct ThreadId(pub String);               // newtype: transparent

#[derive(SemanticType, IntoValue, FromValue, Clone, Copy, Debug, PartialEq, Eq)]
#[semantic(rename_all = "snake_case")]
pub enum TurnState { Pending, Running, Completed, Failed, Interrupted, Cancelled }

#[derive(SemanticType, IntoValue, FromValue, Clone, Debug, PartialEq)]
pub struct TokenUsage { pub input: u64, pub output: u64, #[semantic(default)] pub cached: u64 } // record: plain keys

#[derive(SemanticType, IntoValue, FromValue, Clone, Debug, PartialEq)]
#[semantic(tag = "kind", rename_all = "snake_case")]
pub enum EventPayload {                         // internally tagged variant
    MessageDelta { role: String, text: String },
    ToolCall { name: String, input: semantic_data::Value },
    TurnCompleted { usage: TokenUsage },
}
```

### Step 3 - class structs (qualified-key codec)

```rust
#[derive(Class, Clone, Debug, PartialEq)]
#[semantic(id = "semantic:agent:thread")]
pub struct Thread {
    #[semantic(attr = semantic_data::attr::AttrId)] pub id: ThreadId,
    #[semantic(attr = semantic_data::attr::AttrTitle)] pub title: String,   // shared attr
    pub status: ThreadStatus,                    // key semantic:agent:thread:status
    #[semantic(attr = semantic_data::attr::AttrCreatedAt)] pub created_at: DateTime,
    pub last_seq: u64,                           // key semantic:agent:thread:last_seq
    pub subject: Option<String>,                 // Ref(any) in the schema, String in Rust
}
```

For `Ref` attributes the Rust type is `String`; the registered
`AttributeType` carries `TypeKind::Ref(EntityRef::new(..).with_on_delete(..))`
(like `AttrParent`'s `schema = parent_attribute()`).

### Step 4 - current schema module (`schema.rs`)

Hand-written constants + builders, as `tasks/schema.rs`:

```rust
pub const PACKAGE_NAME: &str = "semantic.agent";
pub const MODULE_NAME: &str = "v1";
pub const COLLECTION: &str = "semantic_agents";
pub const EVENTS_COLLECTION: &str = "semantic_agent_events";
pub const THREAD_CLASS_ID: &str = "semantic:agent:thread";
pub const ATTR_THREAD_STATUS: &str = "semantic:agent:thread:status";
pub fn attributes() -> Vec<AttributeType> { /* helpers::attribute(id, name, T::semantic_type()) */ }
pub fn classes() -> Vec<ClassType> { /* strict_schema: true, creatable_in_ui: Some(false), ... */ }
```

Add a consistency test (copy jobs): `Thread::semantic_type()` record fields
equal the class attribute ids and `required` flags; every encoded key is a
declared attribute; `Thread::from_value(thread.into_value())` round-trips.

### Step 5 - frozen v1 snapshot + migration (`migration_v1.rs`)

* Copy-paste (do **not** call `schema::*`) the attribute/class builders into
  `migration_v1.rs` (helpers frozen as in
  `crates/base/src/migration_support_v1.rs`). For enum attribute types write
  the variant list literally (`string_enum(&["pending", ...])`), do not call
  `TurnState::semantic_type()`: adding a variant to the Rust enum later must not
  retroactively change migration 001.
* Operations: `UpsertAttribute*`, `UpsertClass*`, `UpsertCollection`
  (`Polymorphic`, `StrictRegisteredSchema`), `UpsertIndex*` (qualified field
  ids; composite unique `(thread, seq)` with `kind: Range`, `extra_fields:
  vec![ATTR_EVENT_SEQ]`), optional `UpsertRelationship`.
* `Migration { module: "v1".into(), name: "001_init".into(), description,
  operations, meta: Meta::default() }`.

### Step 6 - `package()`

```rust
pub fn package() -> Package {
    Package { name: PACKAGE_NAME.into(),
        root: Module { name: MODULE_NAME.into(), attributes: ..., classes: ..., ..empty },
        modules: BTreeMap::from([(shared::MODULE_NAME.into(), shared::module())]), // if reusing shared attrs
        migrations: vec![shared::migration_v1(), migration_v1::migration()],
        version: None, meta: Meta::default() }
}
```

`root` = **current** definitions; each later change = append `002_*` and
update `schema.rs` so `root` still equals the replay result.

### Step 7 - tests (mirror existing)

1. `validate_package_migrations(&package())` passes (module == replay).
2. Round trip against a real in-memory DB:

   ```rust
   let db = semantic_db_core::Db::new(semantic_db_core::embedded::EmbeddedBackend::new(
       semantic_db_kv::open_memory().unwrap()));
   db.upsert_package(package()).await.unwrap();   // twice => second run executes no migrations
   // insert Thread::into_value(), query by index, decode with Thread::from_value
   ```
3. Hash pin: add the new package's migrations to
   `crates/base/tests/listing_migrations.rs` (`migrations()` list) and regenerate
   `listing_migrations.sha256` **once at the first release**; afterwards CI
   fails on any edit of a shipped migration.
4. Upgrade test: install previous package version, then current; assert only
   the new migrations executed (`PackageRegistrationOutcome.executed_migrations`).
5. Class/codec consistency test (step 4), plus `include_in_ui_listings` /
   `creatable_in_ui` assertions for internal classes.
6. Index/query test: keyset page returns events in `seq` order; duplicate
   `Create` of the same `(thread, seq)` id fails with `EntityExists`.

### Step 8 - runtime package and app wiring

* Domain crate: `AgentContext` trait + `AgentPackage: RuntimePackage` (see
  section 8). Service layer generic over a store trait (like `LabelStore`) so it
  is unit-testable without the app.
* App crate: `impl AgentContext for AppRequestContext`, `AppConfig::
  agents_enabled` + `SEMANTIC_AGENTS`, `register_package(AgentPackage)` in
  `build()`, capability flag, `sdk_export` target.
* Explicit scopes: document that the package must be installed with
  `semantic.db.package.upsert` (or the scope init) before use; commands should
  return an "unavailable" error rather than auto-migrate unrelated scopes (as
  tasks do).

### Step 9 - schema evolution rules for later

* Add attribute: new `UpsertAttribute` + `UpsertClass` (full replacement of the
  class) in `002_*`; make the field optional or provide `default` so existing
  rows stay valid (validation is opt-in but migrations are re-validated once
  activated).
* Rename/retype: add new attribute, backfill with `MigrationOperation::Update`,
  keep reading old key as decode alias for a release; never edit 001.
* Keep `002` builders frozen the same way (copy, don't reference current).

---

## 10. Recommendations and constraints for modeling agent sessions, turns, events

These are data-layer recommendations; provider/CLI protocol design is covered by
other research notes.

### 10.1 Layered model

1. **Entities (low frequency, mutable, UI-listed in agent views only):**
   * `semantic:agent:thread` (a.k.a. session): `title` (shared), `provider`
     (enum/string), `model`, `runtime_mode`, `workspace_root` (string path),
     `status` (`idle running waiting_approval error closed`), `active_turn`
     (id string, optional), `provider_session_id` (opaque resume token),
     `last_seq` (u64, highest committed event seq), `subject` (Ref any,
     optional), `semantic:parent` (fork parent), `created_at`, `updated_at`,
     `archived`.
   * `semantic:agent:turn`: `thread` (Ref -> thread, Cascade), `index`
     (u32/u64), `state`, `prompt` (string snapshot; or file ref when large),
     `started_at`, `completed_at`, `usage` (record), `error` (record),
     `first_seq`/`last_seq` (u64) to locate its events.
2. **Events (high frequency, append-only, immutable)** in a *separate
   collection* `semantic_agent_events` (polymorphic or `Schema`, strict):
   * One class `semantic:agent:event`: `thread` (string/Ref), `seq` (u64),
     `turn` (string, optional), `at` (DateTime), `kind` (enum), `payload`
     (tagged variant or `Json`/`Object`), optional `blob` (Ref to filestore
     file) for large payloads.
   * Entity id = deterministic `"{thread_id}:{seq:016x}"` and written with
     `BatchOperation::Create` => duplicate/replayed appends fail with
     `EntityExists` (idempotency), and key order == time order.
   * Indexes: unique composite Range `(thread, seq)` (`UpsertIndex { unique:
     true, kind: Range, field: thread, extra_fields: [seq] }`), Equality on
     `turn`. Optional FullText on a `text` attribute for transcript search.
3. **Projections (read models) maintained by the orchestrator in the same
   batch** as the events they summarize: thread `status/last_seq/updated_at`,
   turn state/usage, and optionally a materialized `message` entity per
   completed assistant/user message (role, final text, turn, seq range) so list
   and search views never replay deltas. Keep events as the source of truth and
   projections rebuildable (store an `projection_version`).

Use the `event.kind` + tagged payload approach rather than a class per event
type: queries are exact-`type`, there is no subclass query, one collection and
one index serve all kinds, and new kinds are additive in the payload variant
(a forward migration replaces the attribute type).

### 10.2 Typed payloads via derive

* Event payload enum with `#[semantic(tag = "kind")]` yields an
  InternallyTagged `VariantType`; attribute type = that variant, so the
  catalog validates the shape on write when validation is active.
* Records (token usage, tool call args) as plain-key records. Free-form
  provider payloads (unmodeled tool inputs) as `Value`/`TypeKind::Json`
  (`SemanticType for Value` => `Any`), not as ad-hoc stringified JSON.
* Do not put raw provider JSON in attributes of the common model beyond a
  small `raw` escape hatch; keep a stable common vocabulary.
* All class fields get explicit qualified ids derived from the class id; share
  `title`/`created_at`/`updated_at`/`parent` through the builtin markers.

### 10.3 Ids and ordering

* Thread/turn ids: UUID v4 strings (jobs precedent) or prefixed ids
  (`thread-<uuid>`). Event ids deterministic `thread:seq` as above.
* `seq` is a **per-thread monotonically increasing `u64`** assigned by the
  single in-process orchestrator owner of that thread; persist `last_seq` on the
  thread in the same batch to recover after restart. Always use the same integer
  width (`u64`) on writes, index keys and query literals (known mixed-width
  comparison caveat; SQL literals are I64: prefer bound parameters/AST values
  with explicit `Value::U64`).
* Do not use wall-clock as the order key (clock skew, equal timestamps); keep
  `at` as data.

### 10.4 Write path for streaming

* Never commit per token. Coalesce deltas per `(thread, turn, stream)` into one
  `message_delta` event per 100-250 ms window or ~2-4 KB, whichever first, and
  commit several events + the thread/turn projection update in **one `Batch`**.
  Target <= ~5-10 commits/s per active thread (redb fsync commits are tens of
  ms for 100-row batches in the benchmark; estimate only, measure with
  `crates/db_bench` style workload before fixing numbers).
* Flush immediately on semantic boundaries (tool call start/end, approval
  request, turn end, error, process exit) and on shutdown.
* Final assistant message: write the complete text once at turn end (message
  projection) so replay never needs to concatenate hundreds of deltas.
* Large content (tool stdout, diffs, file snapshots, image attachments >
  ~16-64 KB, an arbitrary threshold to be tuned): store through the file service
  (`semantic:filestore:file` + object store; create-only, hash-addressed) and
  keep a `Ref` + preview/size/hash in the event. This also keeps entity rows
  small since `Upsert` rewrites whole rows.
* Updates to mutable rows (thread status) are `Upsert` of the small thread row
  (or predicate `Update` assignment). Because conflict detection is global,
  serialize per-thread writers with an async mutex and keep the batch the unit
  of atomicity (precedent: `static WRITES: futures::lock::Mutex`).
* Use `execute_batch_returning(batch, BatchReturn::Stats)`; avoid returning
  datasets.
* Backpressure: bounded `mpsc` between provider reader and writer task; if the
  writer lags, coalesce harder rather than drop events.

### 10.5 Relations and references

* `thread.subject`: `Ref(any)` (like `semantic:parent`) or relation entity
  `semantic:agent:entity_thread` (indexed, `entity_collection` attr), if one
  subject needs many threads and queries "threads for entity X" are frequent.
  The comments package shows the full pattern including `RelationType`
  registration (`UpsertRelationship`) and collection-qualified identity.
* `turn.thread` / `event.thread`: `Ref` with `OnDelete::Cascade` gives atomic
  transitive deletion of a thread's turns/events. Be careful: cascade deletes
  of very large event sets run in one transaction, and Refs add reverse-
  reference maintenance on every event insert once validation is active.
  Alternative: store `thread` as a plain string on events, index it, and delete
  by predicate in bounded chunks (`DeleteQuery` with limit) from a maintenance
  routine. Decide by measurement.
* Forks/subagents: `semantic:parent` on thread (shared attribute) plus
  optional `fork_seq`.
* Attachments/files: `Ref` to `semantic:filestore:file`.
* Task link: section 5.

### 10.6 Recovery and lifecycle

* On startup (and scope open), the orchestrator reconciles: any thread in
  `running/waiting_approval` or turn in `pending/running` becomes
  `interrupted` with a terminal event appended (jobs does the same with
  `Interrupted`). Provider-side resume is a *new* turn/command that uses the
  stored `provider_session_id`, never an implicit replay.
* Persist approvals/pending questions as events (and a projection) so UI reload
  during `waiting_approval` can answer them; the live process state is not
  persisted.
* Scope lifetime: the supervisor must pin the scope DB while sessions run and
  shut down cleanly (new `ScopeManager` seam; jobs/plugins are special-cased
  today). Do not rely on `Db` handles being retained by idle retirement.
* Retention: the jobs history cap does not apply; provide explicit archive/
  delete commands for threads (bounded chunk deletion) and optional event
  compaction (replace delta runs by a final message event) as a forward feature.

### 10.7 Live updates to the UI

* Snapshot + tail protocol over a typed stream command, e.g.
  `semantic.agent.thread.subscribe { thread_id, after_seq } ->
  StreamOf<AgentEvent, ()>`: server reads events with `seq > after_seq`
  (keyset on the unique index), then switches to the in-process broadcast,
  de-duplicating by `seq`; on `Lagged` the server re-reads from the DB.
  Ephemeral *pre-commit* token deltas can be sent on the stream before they are
  coalesced into a stored event (`seq` unassigned / marked `ephemeral`), so
  the UI feels live without per-token commits. Persisted events are the replay
  authority.
* Provide a unary `semantic.agent.thread.events` (keyset paged) for initial
  load and non-streaming clients (polling fallback consistent with jobs).
* If a generic DB change stream is wanted later, the additive path is to add
  default-`Unsupported` `subscribe_changes` to `SemanticDb` (and a typed
  stream command) and bridge `Db::subscribe_changes`; it is additive but
  touches the app facade and is only supported by embedded engines. It would
  also expose raw row payloads for the events collection; filter by
  `collections: ["semantic_agent_events"]` + `include_payloads`.

### 10.8 UI exposure hygiene

* Events, turns: `creatable_in_ui: Some(false)`, `include_in_ui_listings:
  Some(false)`. Threads: hide from generic `/browse` too unless a generic
  entity page is desired; give them a custom renderer in
  `ui_core/src/ui_catalog` like tasks.
* Avoid dumping giant payloads into generic entity views (a renderer with
  truncation is needed or exclude the class).

---

## 11. Gaps, risks and decisions needed (do not assume)

1. **Schema-from-Rust derive.** Decide: (a) hand-written `ClassType` + codec
   consistency test (matches jobs/tasks today, zero core change), or (b) extend
   `crates/data` + `crates/macros` with a `ClassSchema` trait/derive that
   produces `ClassType`/`AttributeType`. (b) is a core, user-visible change
   (AGENTS: ask before changing core behavior). If (b), migrations still need
   frozen snapshots; the derive must only feed the *current* module.
2. **Package home.** New crate `semantic_agent_core` (data model + package,
   wasm-safe) and `semantic_agent` (orchestration, tokio) vs. putting the
   package in `crates/data`. A separate crate keeps `crates/data` free of
   product-specific schema; but note `semantic_sdk_export` and the hash-pin
   test list need edits.
3. **Change feed exposure** (see 10.7). Needs an app-facade addition or an
   in-process bus; Postgres has no feed.
4. **Interactive transactions** are not on `SemanticDb`; the orchestration crate
   either lives with `Db` access (embedded only) or sticks to atomic batches +
   in-process serialization. Prefer the latter (portable across backends).
5. **Generic scope-service lifecycle** for the supervisor (pin DB, shutdown
   ordering) is not available; requires an app change (new seam or
   special-casing like `jobs`).
6. **Validation is opt-in** per DB; the service layer must validate agent
   invariants (seq monotonicity, state machine, required fields). Refs to
   non-existent entities are only rejected where validation/FK checking is
   active (`WriteSettings::validate_foreign_keys` defaults to true; confirm
   enforcement semantics before relying on it).
7. **Postgres backend**: no change feed, no interactive transactions
   (`Unsupported`); index/query support for composite range + full text should
   be verified per backend before promising them (db_test suite
   `crates/db_test/src/suite` runs on memory, redb, logfs).
8. **`db_log` (WAL) engine**: replay cost grows with WAL size and collection
   rewrites generate large events; avoid for transcript-heavy deployments or
   benchmark first.
9. **Integer comparison caveat** (deferred `Value::cmp` fix): use `u64`
   consistently for `seq` in data, index and query.
10. **Entity size**: no documented limit; keep event rows small and push
    blobs to the filestore. Run a size/throughput benchmark with the
    `crates/db_bench` harness for the agent event shape before committing to
    coalescing parameters.
11. **Tasks not in the migration hash-pin test list**; add agent (and tasks) when
    touching it.
12. **`SemanticType` gaps** (`Date`, `Uuid`, `Bytes`, `Duration`, `HashMap`,
    `u128`) will force wrappers; consider adding the missing impls to
    `crates/data` (additive) if agent types need them.
13. **Plugin angle.** Providers (Claude Code, Codex, ...) can start as Rust
    trait implementations inside the orchestration crate; the plugin system
    supports Rust/stdio/WebSocket providers exporting named interfaces
    (`semantic.import/v1` is the only one shipped) and persists activation
    records in `semantic.plugin`. Third-party plugins cannot ship persisted
    schema/migrations today, so provider-specific data must fit the common
    model (typed payload + `Value` escape hatch).

---

## 12. File index (quick reference)

Data model:
`crates/data/src/value/{val,obj,convert}.rs`, `crates/data/src/attr/mod.rs`,
`crates/data/src/builtin.rs`, `crates/data/src/schema/{core,class,attribute,
relation,index,migration,package,module,variant,enum}/*`,
`crates/data/src/query/mod.rs` (Batch, UpdateQuery, FieldFormat),
`crates/data/src/{jobs,filestore,plugin,import}.rs`,
`crates/data/src/bundles/{shared,directory,auth,query}`.

Derives: `crates/macros/src/{lib,model}.rs`, `crates/data/tests/derive.rs`,
`crates/data/src/value/convert.rs` (`__private` helpers).

Migrations/packages: `crates/db_core/src/{managed_schema,ddl}.rs`,
`crates/db_core/src/embedded/db.rs::apply_package_update`,
`crates/db_core/src/embedded/db/package_registration_tests.rs`,
`crates/base/src/{bundle,migrations,migration_support_v1,domain_support}*`,
`crates/base/src/tasks/{schema,migration_v1,model,service,commands,mod}.rs`,
`crates/base/src/comments/*`, `crates/base/tests/{listing_migrations.rs,
listing_migrations.sha256,package_registration.rs}`,
`crates/app/src/{command,scope,capabilities,config,task_comments}.rs`,
`crates/rpc_core/src/package.rs`.

DB runtime: `crates/db_core/src/{backend,change_feed,transaction,config,
batch_return}.rs`, `crates/app/src/db.rs`, `crates/db_redb/src/options.rs`,
`docs/{benchmarks,validation,testing,file-maintenance}.md`,
`crates/rpc/src/stream_command.rs`, `crates/rpc/src/client.rs`.

Jobs: `crates/jobs/src/*`, `crates/app/src/jobs/*`,
`docs/plans/2026-09-10-jobs-system/design.md`.

Plans: `docs/plans/2026-09-30-task-system/{plan,implementation,research}.md`,
`docs/plans/2026-09-09-plugin-system/design.md`,
`docs/plans/2026-10-03-plugin-dbs/plan.md`.
