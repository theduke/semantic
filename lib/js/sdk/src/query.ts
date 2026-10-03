import type {
  AggregateOp,
  Assignment,
  BinaryOp,
  DeleteQuery,
  DdlOperation,
  Expr,
  FieldPath,
  InsertQuery,
  OrderBy,
  Query,
  QueryField,
  SelectQuery,
  SemanticObject,
  SemanticValue,
  UnaryOp,
  UpdateQuery,
} from "./types.js";

/** Each string is one field name, including names containing dots or colons. */
export const fieldPath = (...segments: Array<string | number>): FieldPath =>
  segments.map((segment) => {
    if (typeof segment === "string") return { field: segment };
    if (!Number.isSafeInteger(segment) || segment < 0)
      throw new RangeError(
        "field path indices must be nonnegative safe integers",
      );
    return { index: segment };
  });

const binary = (op: BinaryOp, left: Expr, right: Expr): Expr => ({
  binary: { op, left, right },
});
const literal = (value: SemanticValue): Expr => ({
  operand: { literal: value === undefined ? { $tagged: "void" } : value },
});
const combine = (op: "and" | "or", expressions: Expr[]): Expr =>
  expressions.length
    ? expressions
        .slice(1)
        .reduce((left, right) => binary(op, left, right), expressions[0]!)
    : literal(op === "and");

/** Typed expression constructors. Literal values use the ordinary Semantic value codec. */
export const expr = {
  field: (...segments: Array<string | number>): Expr => ({
    operand: { field: fieldPath(...segments) },
  }),
  literal,
  parameter: (name: string): Expr => {
    if (!/^[A-Za-z_][A-Za-z0-9_]*$/.test(name))
      throw new TypeError(
        "parameter names must be ASCII identifiers without a colon",
      );
    return { operand: { parameter: name } };
  },
  binary,
  unary: (op: UnaryOp, expression: Expr): Expr => ({
    unary: { op, expr: expression },
  }),
  eq: (left: Expr, right: Expr): Expr => binary("eq", left, right),
  ne: (left: Expr, right: Expr): Expr => binary("not_eq", left, right),
  lt: (left: Expr, right: Expr): Expr => binary("lt", left, right),
  lte: (left: Expr, right: Expr): Expr => binary("lte", left, right),
  gt: (left: Expr, right: Expr): Expr => binary("gt", left, right),
  gte: (left: Expr, right: Expr): Expr => binary("gte", left, right),
  and: (...expressions: Expr[]): Expr => combine("and", expressions),
  or: (...expressions: Expr[]): Expr => combine("or", expressions),
  not: (expression: Expr): Expr => ({ unary: { op: "not", expr: expression } }),
  inList: (expression: Expr, list: Expr[], negated = false): Expr => ({
    in_list: { expr: expression, list, negated },
  }),
  isNull: (expression: Expr, negated = false): Expr => ({
    is_null: { expr: expression, negated },
  }),
  coalesce: (...expressions: Expr[]): Expr => ({ coalesce: expressions }),
  call: (name: string, ...args: Expr[]): Expr => ({
    function: { name, args: args.map((expression) => ({ expr: expression })) },
  }),
  aggregate: (op: AggregateOp, expression?: Expr, distinct = false): Expr => ({
    aggregate: {
      op,
      distinct,
      arg: expression === undefined ? "wildcard" : { expr: expression },
    },
  }),
  subquery: (select: SelectQuery): Expr => ({ subquery: select }),
  exists: (select: SelectQuery, negated = false): Expr => ({
    exists: { query: select, negated },
  }),
};

/** Projection helpers, including wildcard expansion of a nested object. */
export const projection = {
  field: (expression: Expr, alias?: string): QueryField => ({
    expr: expression,
    ...(alias === undefined ? {} : { alias }),
  }),
  wildcard: (...segments: Array<string | number>): QueryField => ({
    expr: expr.field(...segments),
    wildcard: fieldPath(...segments),
  }),
};
export const orderBy = (
  expression: Expr,
  direction: "asc" | "desc" = "asc",
): OrderBy => ({ expr: expression, direction });
export const assignment = (path: FieldPath, value: Expr): Assignment => ({
  path,
  value,
});

/** Build queries directly. Omitted fields use the defaults declared by the Semantic schema. */
export const query = {
  select: (
    collection: string | null = null,
    options: Omit<SelectQuery, "collection"> = {},
  ): { select: SelectQuery } => ({ select: { collection, ...options } }),
  insert: (
    collection: string | null,
    objects: SemanticObject[],
    options: Omit<InsertQuery, "collection" | "source"> = {},
  ): { insert: InsertQuery } => ({
    insert: { collection, source: { objects }, ...options },
  }),
  insertValues: (
    collection: string | null,
    columns: string[],
    values: Expr[][],
    options: Omit<InsertQuery, "collection" | "columns" | "source"> = {},
  ): { insert: InsertQuery } => ({
    insert: { collection, columns, source: { values }, ...options },
  }),
  update: (
    collection: string | null,
    assignments: Assignment[],
    options: Omit<UpdateQuery, "collection" | "assignments"> = {},
  ): { update: UpdateQuery } => ({
    update: { collection, assignments, ...options },
  }),
  delete: (
    collection: string | null,
    options: Omit<DeleteQuery, "collection"> = {},
  ): { delete: DeleteQuery } => ({ delete: { collection, ...options } }),
  ddl: (...operations: DdlOperation[]): Extract<Query, { ddl: unknown }> => ({
    ddl: { batch: { operations } },
  }),
};
