/** JSON-like values used by entity objects. Integers may be bigint to retain RPC precision. */
export type SemanticPrimitive =
  | undefined
  | null
  | boolean
  | number
  | bigint
  | string
  | Uint8Array
  | Date;
export type SemanticValue =
  | SemanticPrimitive
  | SemanticValue[]
  | SemanticObject
  | SemanticMap
  | SemanticVariant
  | EncodedTaggedValue;
export interface SemanticObject {
  [field: string]: SemanticValue;
}
export type SemanticMap = Map<SemanticValue, SemanticValue>;
export interface SemanticVariant {
  $variant: string;
  value: SemanticValue;
  type?: string;
}
/** Explicitly typed wire scalar produced by the `value` helpers. */
export interface EncodedTaggedValue {
  readonly $tagged: TaggedValue;
}

/** Exact reflected schema-expression type used by computed/default expressions. */
export type SchemaExpression = import("./generated/core.js").expression_Expr;
/** Exact reflected multi-field and attribute class constraints. */
export type ClassConstraint = import("./generated/core.js").ClassConstraint;
/** Exact reflected DDL batch accepted by an RPC DDL query. */
export type DdlBatch = import("./generated/core.js").DdlBatch;
/** Exact reflected DDL operation accepted by a package migration. */
export type MigrationDdlOperation =
  import("./generated/core.js").MigrationDdlOperation;

export type IntegerTag =
  | "i8"
  | "i16"
  | "i32"
  | "i64"
  | "i128"
  | "u8"
  | "u16"
  | "u32"
  | "u64"
  | "u128";
export type TaggedValue =
  | "void"
  | "null"
  | { bool: boolean }
  | { i8: number }
  | { i16: number }
  | { i32: number }
  | { i64: bigint }
  | { i128: bigint }
  | { u8: number }
  | { u16: number }
  | { u32: number }
  | { u64: bigint }
  | { u128: bigint }
  | { f32: number }
  | { f64: number }
  | { uuid: string }
  | { ip_addr: string }
  | { duration: bigint }
  | { time: bigint }
  | { date: number }
  | { date_time: bigint }
  | { bytes: number[] }
  | { string: string }
  | { list: TaggedValue[] }
  | { map: [TaggedValue, TaggedValue][] }
  | { object: Record<string, TaggedValue> }
  | { variant: { type?: string; variant: string; value: TaggedValue } };

export type PathSegment = { field: string } | { index: number };
export type FieldPath = PathSegment[];
export type ValidationError = import("./generated/core.js").ValidationError;
export type ValidationViolation = import("./generated/core.js").ValidationViolation;
export type BinaryOp =
  | "add"
  | "sub"
  | "mul"
  | "div"
  | "mod"
  | "concat"
  | "and"
  | "or"
  | "eq"
  | "not_eq"
  | "lt"
  | "lte"
  | "gt"
  | "gte"
  | "in";
export type UnaryOp = "not" | "neg";
export type AggregateOp = "count" | "sum" | "avg" | "min" | "max";
export type FunctionArg<T> = { expr: T } | "wildcard";
export type Operand = { field: FieldPath } | { literal: SemanticValue };
export type Expr =
  | { operand: Operand }
  | { unary: { op: UnaryOp; expr: Expr } }
  | { binary: { op: BinaryOp; left: Expr; right: Expr } }
  | { if_else: { cond: Expr; then_expr: Expr; else_expr: Expr } }
  | { coalesce: Expr[] }
  | { function: { name: string; args: FunctionArg<Expr>[] } }
  | {
      aggregate: { op: AggregateOp; distinct: boolean; arg: FunctionArg<Expr> };
    }
  | { in_list: { expr: Expr; list: Expr[]; negated: boolean } }
  | { subquery: SelectQuery }
  | { between: { expr: Expr; low: Expr; high: Expr; negated: boolean } }
  | {
      pattern_match: {
        kind: "like" | "similar_to";
        expr: Expr;
        pattern: Expr;
        case_insensitive: boolean;
        negated: boolean;
      };
    }
  | {
      regex_match: {
        expr: Expr;
        pattern: Expr;
        case_insensitive: boolean;
        negated: boolean;
      };
    }
  | { is_null: { expr: Expr; negated: boolean } }
  | { exists: { query: SelectQuery; negated: boolean } }
  | {
      relation_exists: {
        relation: Expr;
        source: Expr;
        target: Expr;
        transitive: boolean;
        max_depth: Expr | null;
      };
    };
export type FieldFormat = "plain" | "qualified" | "underscore";
export interface QueryField {
  expr: Expr;
  alias: string | null;
  wildcard: FieldPath | null;
}
export interface OrderBy {
  expr: Expr;
  direction: "asc" | "desc";
}
export interface JoinSource {
  collection: string | null;
  class: string | null;
}
export type JoinCondition =
  | { on_expr: Expr }
  | { using_fields: { left: FieldPath; right: FieldPath } };
export interface JoinQuery {
  source: JoinSource;
  alias: string | null;
  join_type: "inner" | "left" | "right" | "full";
  condition: JoinCondition;
  predicate: Expr | null;
}
export interface SelectQuery {
  collection: string | null;
  source_alias: string | null;
  joins: JoinQuery[];
  predicate: Expr | null;
  projection: QueryField[];
  distinct: boolean;
  group_by: Expr[];
  having: Expr | null;
  order_by: OrderBy[];
  offset: Expr;
  limit: Expr | null;
  field_format: FieldFormat;
}
export type InsertSource =
  | { objects: SemanticObject[] }
  | { values: Expr[][] }
  | { select: SelectQuery };
export interface InsertQuery {
  collection: string | null;
  columns: string[];
  source: InsertSource;
  returning: QueryField[];
  field_format: FieldFormat;
}
export interface Assignment {
  path: FieldPath;
  value: Expr;
}
export interface UpdateQuery {
  collection: string | null;
  predicate: Expr | null;
  assignments: Assignment[];
  limit: Expr | null;
  returning: QueryField[];
  field_format: FieldFormat;
}
export interface DeleteQuery {
  collection: string | null;
  predicate: Expr | null;
  limit: Expr | null;
  returning: QueryField[];
  field_format: FieldFormat;
}
export interface DdlQuery {
  batch: DdlBatch;
}
export type Query =
  | { select: SelectQuery }
  | { insert: InsertQuery }
  | { update: UpdateQuery }
  | { delete: DeleteQuery }
  | { ddl: DdlQuery };
export type QueryInput =
  | { ast: Query }
  | { text: { format: "sql" | "prql"; query: string } };

export interface Deprecation {
  note: string | null;
}
export interface Meta {
  title: string | null;
  description: string | null;
  id: string | null;
  deprecated: Deprecation | null;
  aliases: string[];
  examples: SemanticValue[];
  tags: string[];
  docs_url: string | null;
  annotations: Record<string, string>;
}
export interface TypeRef {
  name: string;
  args: TypeNode[];
}
export interface TypeNode {
  kind: TypeKind;
  constraints: Constraint[];
  annotations: Annotation[];
}
export type IntWidth =
  | "i8"
  | "i16"
  | "i24"
  | "i32"
  | "i40"
  | "i48"
  | "i56"
  | "i64"
  | "i128"
  | "i256";
export type UIntWidth =
  | "u8"
  | "u16"
  | "u24"
  | "u32"
  | "u40"
  | "u48"
  | "u56"
  | "u64"
  | "u128"
  | "u256";
export type FloatWidth =
  | "f16"
  | "f32"
  | "f64"
  | "f80"
  | "f128"
  | "decimal32"
  | "decimal64"
  | "decimal128";
export type NumberType =
  | { int: IntWidth }
  | { uint: UIntWidth }
  | { float: FloatWidth }
  | { big_int: { min_bits: number | null; max_bits: number | null } }
  | { big_uint: { min_bits: number | null; max_bits: number | null } }
  | {
      decimal: {
        precision: number | null;
        scale: number | null;
        encoding: string;
      };
    }
  | {
      rational: {
        numerator: NumberType | null;
        denominator: NumberType | null;
      };
    }
  | { complex: { component: FloatWidth } }
  | "unspecified";
export interface StringType {
  format: StringFormat | null;
  normalization: "nfc" | "nfd" | "nfkc" | "nfkd" | null;
}
export type StringFormat =
  | "email"
  | "uri"
  | "url"
  | "hostname"
  | "regex"
  | "uuid"
  | "base64"
  | "hex"
  | "ascii"
  | "utf8"
  | "json_pointer"
  | "json_path"
  | "sql"
  | { custom: string };
export interface BytesType {
  encoding: "raw" | "base64" | "base64_url" | "hex" | "ascii85" | null;
}
export type TimeZoneSpec =
  | "required"
  | "forbidden"
  | "allowed"
  | { specific: string };
export type TemporalType =
  | "date"
  | "time"
  | "date_time"
  | "duration"
  | "period"
  | "instant"
  | {
      timestamp: {
        unit: "seconds" | "millis" | "micros" | "nanos";
        timezone: TimeZoneSpec;
      };
    };
export interface OptionalType {
  inner: TypeNode;
}
export interface ArrayType {
  items: TypeNode;
  length: LengthSpec | null;
}
export interface ListType {
  items: TypeNode;
}
export interface TupleType {
  items: TypeNode[];
  rest: TypeNode | null;
}
export interface MapType {
  keys: TypeNode;
  values: TypeNode;
  ordered: boolean;
}
export interface SetType {
  items: TypeNode;
}
export interface UnionType {
  variants: TypeNode[];
}
export interface IntersectionType {
  variants: TypeNode[];
}
export interface ResultType {
  ok: TypeNode;
  err: TypeNode;
}
export interface EnumVariant {
  name: string;
  value: number | bigint | null;
  symbol: string | null;
  meta: Meta;
}
export interface EnumType {
  repr: "string" | "int";
  variants: EnumVariant[];
}
export type VariantPayload =
  | "unit"
  | { tuple: TypeNode[] }
  | { record: RecordType }
  | { newtype: TypeNode };
export type VariantTag =
  | "externally_tagged"
  | "untagged"
  | { internally_tagged: { field: string } }
  | { adjacently_tagged: { tag_field: string; data_field: string } };
export interface VariantCase {
  name: string;
  payload: VariantPayload;
  discriminant: SemanticValue | null;
  meta: Meta;
}
export interface VariantType {
  tag: VariantTag;
  variants: VariantCase[];
}
export interface StreamType {
  element: TypeNode;
  end: TypeNode | null;
}
export interface HandleType {
  interface: TypeRef;
  mode: "own" | "borrow";
}
export interface OpaqueType {
  id: string;
  domain: string | null;
  repr: string | null;
}
export interface ExtensionType {
  namespace: string;
  name: string;
  payload: Record<string, string>;
}
export type TypeKind =
  | "uuid"
  | "json"
  | { any: Record<string, never> }
  | { never: Record<string, never> }
  | { unknown: Record<string, never> }
  | { null: Record<string, never> }
  | { bool: Record<string, never> }
  | { char: { unicode_scalar: boolean } }
  | { number: NumberType }
  | { string: StringType }
  | { bytes: BytesType }
  | { temporal: TemporalType }
  | { ip_addr: "v4" | "v6" | "any" }
  | { optional: OptionalType }
  | { array: ArrayType }
  | { list: ListType }
  | { tuple: TupleType }
  | { map: MapType }
  | { set: SetType }
  | { record: RecordType }
  | { attribute: AttributeType }
  | { class: ClassType }
  | { union: UnionType }
  | { intersection: IntersectionType }
  | { variant: VariantType }
  | { enum: EnumType }
  | { result: ResultType }
  | { function: FunctionType }
  | { interface: InterfaceType }
  | { handle: HandleType }
  | { stream: StreamType }
  | { opaque: OpaqueType }
  | { extension: ExtensionType }
  | { ref: TypeRef };
export type AnnotationValue =
  | { bool: boolean }
  | { number: string }
  | { string: string }
  | { list: AnnotationValue[] }
  | { map: Record<string, AnnotationValue> };
export interface Annotation {
  key: string;
  value: AnnotationValue;
}
export type LengthSpec =
  | { exactly: number | bigint }
  | { range: { min: number | bigint | null; max: number | bigint | null } };
export type NumberBound = { inclusive: string } | { exclusive: string };
export type Constraint =
  | { min: NumberBound }
  | { max: NumberBound }
  | { multiple_of: string }
  | { length: LengthSpec }
  | { pattern: string }
  | { prefix: string }
  | { suffix: string }
  | { contains: SemanticValue }
  | { precision: { precision: number; scale: number } }
  | { charset: string | { custom: string } }
  | { collation: string }
  | { time_zone: TimeZoneSpec }
  | { min_items: number | bigint }
  | { max_items: number | bigint }
  | { min_properties: number | bigint }
  | { max_properties: number | bigint }
  | { required_fields: string[] }
  | { key_pattern: string }
  | "unique"
  | "distinct"
  | "primary_key"
  | { foreign_key: { to: TypeRef; fields: string[] } }
  | { index: { name: string | null; fields: string[]; unique: boolean } }
  | { default_value: { value: SemanticValue } }
  | { default_expr: { expr: SchemaExpression } }
  | {
      transport: {
        format: string | { custom: string };
        media_type: string | null;
      };
    };
export interface TypeParam {
  name: string;
  bounds: TypeRef[];
  default: TypeNode | null;
}
export interface TypeDef {
  name: string;
  module: string | null;
  params: TypeParam[];
  ty: TypeNode;
  visibility: "public" | "internal" | "private";
  meta: Meta;
}
export interface RecordField {
  ty: TypeNode;
  required: boolean;
  readonly: boolean;
  writeonly: boolean;
  default: SemanticValue | null;
  meta: Meta;
}
export interface RecordType {
  fields: Record<string, RecordField>;
  open: boolean;
  additional: TypeNode | null;
  required_order: string[] | null;
}
export interface AttributeType {
  id: string;
  name: string;
  ty: TypeNode;
  constraints: Constraint[];
  meta: Meta;
}
export interface AttributeRef {
  id: string;
}
export interface ClassRef {
  id: string;
}
export interface ClassAttribute {
  attribute: AttributeRef;
  required: boolean;
  ui_order: number | null;
  computed: SchemaExpression | null;
  constraints: Constraint[];
  meta: Meta;
}
export interface ClassType {
  id: string;
  name: string;
  inherits: ClassRef | null;
  extends: ClassRef[];
  "semantic:class:strict_schema": boolean;
  attributes: Record<string, ClassAttribute>;
  constraints: ClassConstraint[];
  meta: Meta;
}
export interface FunctionParam {
  name: string | null;
  ty: TypeNode;
}
export interface FunctionType {
  params: FunctionParam[];
  results: TypeNode[];
  throws: TypeNode | null;
  async_fn: boolean;
}
export interface InterfaceMethod {
  name: string;
  signature: FunctionType;
}
export interface InterfaceType {
  methods: InterfaceMethod[];
}
export interface ContractFunction {
  name: string;
  signature: FunctionType;
  meta: Meta;
}
export interface ContractConstant {
  name: string;
  ty: TypeNode;
  value: SemanticValue;
  meta: Meta;
}
export interface ContractInterface {
  name: string;
  interface: InterfaceType;
  meta: Meta;
}
export interface Contract {
  name: string;
  constants: Record<string, ContractConstant>;
  types: Record<string, TypeDef>;
  functions: Record<string, ContractFunction>;
  attributes: Record<string, AttributeType>;
  classes: Record<string, ClassType>;
  interfaces: Record<string, ContractInterface>;
  meta: Meta;
}
export interface Module {
  name: string;
  constants: Record<string, ContractConstant>;
  types: Record<string, TypeDef>;
  attributes: Record<string, AttributeType>;
  classes: Record<string, ClassType>;
  interfaces: Record<string, InterfaceType>;
  contracts: Record<string, Contract>;
  meta: Meta;
}
export type MigrationOperation =
  | { ddl: MigrationDdlOperation }
  | { insert: { collection: string; id: string; object: SemanticObject } }
  | { update: { query: UpdateQuery } }
  | { delete: { query: DeleteQuery } };
export interface Migration {
  module: string;
  name: string;
  description: string | null;
  operations: MigrationOperation[];
  meta: Meta;
}
export interface SchemaVersion {
  major: number;
  minor: number;
  patch: number;
  pre: string | null;
  build: string | null;
}
export interface Package {
  name: string;
  root: Module;
  modules: Record<string, Module>;
  migrations: Migration[];
  version: SchemaVersion | null;
  meta: Meta;
}

export interface EntityRecord<T extends object = SemanticObject> {
  id: string;
  collection: string;
  object: T;
}
export type QueryResult<T extends object = SemanticObject> =
  | { kind: "select"; rows: T[] }
  | { kind: "insert"; inserted: number | bigint; returning: T[] }
  | { kind: "update"; stats: MutationStats; returning: T[] }
  | { kind: "delete"; deleted: number | bigint; returning: T[] }
  | { kind: "ddl" };
export interface MutationStats {
  matched: number | bigint;
  affected: number | bigint;
}
export interface BatchStats {
  upserted: number | bigint;
  deleted: number | bigint;
  updated: number | bigint;
}
export interface BatchOutcome {
  dataset: Record<string, Record<string, SemanticObject>>;
  stats: BatchStats;
}
export type BatchReturn =
  | "dataset"
  | "stats"
  | "changes"
  | { projection: { fields: string[] } };
export interface EntityChange {
  collection: string;
  id: string;
  kind: "upsert" | "delete";
}
export interface BatchStatsReply {
  stats: BatchStats;
}
export interface BatchChangesReply extends BatchStatsReply {
  changes: EntityChange[];
}
export interface BatchProjectionReply<T extends object = SemanticObject>
  extends BatchChangesReply {
  rows: EntityRecord<T>[];
}
export type BatchReply =
  | BatchOutcome
  | BatchStatsReply
  | BatchChangesReply
  | BatchProjectionReply;
export type BatchReturnResult<R extends BatchReturn> = R extends "dataset"
  ? BatchOutcome
  : R extends "stats"
    ? BatchStatsReply
    : R extends "changes"
      ? BatchChangesReply
      : BatchProjectionReply;
/** Batch operations currently accepted by the application RPC command. */
export type BatchOperation =
  | { kind: "create"; collection: string; id: string; object: SemanticObject }
  | { kind: "upsert"; collection: string; id: string; object: SemanticObject }
  | { kind: "delete_by_id"; collection: string; id: string }
  | { kind: "delete_by_ids"; collection: string; ids: string[] };
export interface ScopeInfo {
  scope_id: string;
  owner: string;
  visibility: "principal" | "system";
  uri: string;
  loaded: boolean;
}
export interface FileAnalysisOutcome {
  id: string;
  collection: string;
  analyzed: boolean;
  analysis_kind: string | null;
  attributes: SemanticObject;
  object: SemanticObject;
}

export interface CommandDefinition<P = SemanticObject, O = SemanticValue> {
  readonly name: string;
  readonly _payload?: P;
  readonly _output?: O;
}
