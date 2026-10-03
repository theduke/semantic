import assert from "node:assert/strict";
import test from "node:test";
import {
  SemanticClient,
  HttpTransport,
  query,
  expr,
  projection,
  orderBy,
  assignment,
  fieldPath,
  value,
  encodeTagged,
  decodeTaggedExact,
  stringifyJson,
  parseJson,
  commands,
  emptyMeta,
  PackageBuilder,
} from "../index.js";
import type {
  Query,
  SemanticObject,
  SemanticValue,
  TaggedValue,
  TypeNode,
} from "../index.js";
import { packageModel, renderPackage } from "../generator/index.js";

const exact = (value: SemanticValue) => stringifyJson(encodeTagged(value));

test("undefined literals encode an explicit Semantic Void", () => {
  assert.equal(
    exact(expr.literal(undefined)),
    stringifyJson({
      object: { operand: { object: { literal: "void" } } },
    }),
  );
});

test("clients construct reusable ASTs and bind parameters without a parse request", async () => {
  const calls: Array<{ command: string; payload: SemanticObject }> = [];
  const client = new SemanticClient({
    async invoke(command, payload, options) {
      assert.equal(options?.signal, signal);
      calls.push({ command, payload: payload as SemanticObject });
      return { kind: "select", rows: [] };
    },
  });
  const signal = new AbortController().signal;
  const ast = query.select("items", {
    projection: [
      projection.field(expr.field("title")),
      projection.wildcard("details"),
    ],
    predicate: expr.and(
      expr.eq(expr.field("owner"), expr.parameter("owner")),
      expr.inList(expr.field("state"), [
        expr.literal("open"),
        expr.literal("done"),
      ]),
    ),
    order_by: [orderBy(expr.field("created_at"), "desc")],
    limit: expr.parameter("limit"),
  });
  const before = exact(ast);
  for (const owner of ["alice", "bob"]) {
    await client.query(ast, {
      params: { owner, limit: value.int("u64", 5) },
      scopeId: "work",
      signal,
    });
  }
  assert.deepEqual(
    calls.map((call) => call.command),
    ["semantic.db.query", "semantic.db.query"],
  );
  assert.equal(exact(ast), before);
  assert.equal(calls[0]!.payload.query, ast);
  assert.equal(calls[1]!.payload.query, ast);
  assert.equal(calls[0]!.payload.scope_id, "work");
  assert.equal(Object.hasOwn(calls[0]!.payload, "format"), false);
  assert.equal((calls[1]!.payload.params as SemanticObject).owner, "bob");
  // Runtime validation also protects untyped JavaScript callers.
  await assert.rejects(
    client.query(ast, { format: "sql" } as never),
    /only supported for text/,
  );
});

test("mutation and DDL builders send canonical externally tagged ASTs", async () => {
  const operations: Query[] = [
    query.insert("items", [{ id: "a", bytes: new Uint8Array([0, 255]) }]),
    query.insertValues(
      "items",
      ["id", "title"],
      [[expr.literal("b"), expr.parameter("title")]],
    ),
    query.update("items", [
      assignment(fieldPath("details", 0, "name"), expr.parameter("name")),
    ]),
    query.delete("items", {
      predicate: expr.eq(expr.field("id"), expr.literal("a")),
    }),
    query.ddl(
      {
        upsert_collection: {
          name: "items",
          kind: "untyped",
          integrity_mode: "permissive",
        },
      },
      {
        upsert_index: {
          collection: "items",
          name: "title",
          field: "title",
          unique: false,
          predicate: expr.eq(expr.field("state"), expr.literal("open")),
        },
      },
    ),
  ];
  const sent: TaggedValue[] = [];
  const client = new SemanticClient({
    async invoke(command, payload) {
      assert.equal(command, "semantic.db.query");
      sent.push(encodeTagged((payload as SemanticObject).query));
      return { kind: "ddl" };
    },
  });
  for (const ast of operations) await client.query(ast);
  assert.deepEqual(
    sent,
    operations.map((ast) => encodeTagged(ast)),
  );
  assert.throws(() => expr.parameter(":name"), /without a colon/);
  assert.throws(() => fieldPath(-1), /nonnegative/);
  assert.deepEqual(expr.aggregate("count"), {
    aggregate: { op: "count", distinct: false, arg: "wildcard" },
  });
});

test("parseSql HTTP results keep exact literals and structural scalars on resubmission", async () => {
  const literals = [
    value.int("u8", 7),
    value.int("u64", 18446744073709551615n),
    value.uuid("0f8fad5b-d9cb-469f-a165-70867728950e"),
    value.timeNanos(999),
    value.dateJulianDay(2460000),
    value.durationMs(3),
    value.dateTimeNanos(123456789),
    { $tagged: { f32: 1.5 } },
    new Uint8Array([0, 255]),
    null,
    true,
    "text",
    value.void(),
    { absent: value.void(), explicit_null: null },
    {
      $tagged: {
        object: {
          $tagged: { string: "ordinary field" },
          $variant: { string: "another field" },
        },
      },
    },
    {
      $tagged: {
        object: { $variant: { string: "ordinary field" }, value: "null" },
      },
    },
    new Map([[value.int("u8", 1), value.void()]]),
    value.variant("case", value.void(), "Kind"),
  ] satisfies SemanticValue[];
  const ast: Query = {
    select: {
      collection: "items",
      distinct: false,
      predicate: expr.eq(expr.field("id"), expr.parameter("id")),
      projection: literals.map((literal, i) =>
        projection.field(expr.literal(literal), `v${i}`),
      ),
      order_by: [
        orderBy({
          operand: {
            field: [{ field: "array" }, { index: value.int("u64", 2) }],
          },
        }),
      ],
    },
  };
  const encoded = encodeTagged(ast);
  const signal = new AbortController().signal;
  let parseCalls = 0;
  let queryCalls = 0;
  const client = new SemanticClient(
    new HttpTransport("https://example.test/rpc", {
      fetch: async (_input, init) => {
        const request = parseJson(String(init?.body)) as {
          id: bigint;
          command: string;
          payload: TaggedValue;
        };
        const payload = decodeTaggedExact(
          request.payload,
          true,
        ) as SemanticObject;
        let output: TaggedValue;
        if (request.command === "semantic.db.query.parse_sql") {
          assert.equal(payload.query, "SELECT :id FROM items");
          assert.equal(payload.scope_id, "work");
          assert.equal(Object.hasOwn(payload, "params"), false);
          parseCalls++;
          output = encoded;
        } else {
          assert.equal(request.command, "semantic.db.query");
          assert.equal(exact(payload.query), stringifyJson(encoded));
          assert.equal(Object.hasOwn(payload, "format"), false);
          queryCalls++;
          output = encodeTagged({ kind: "select", rows: [] });
        }
        return new Response(
          stringifyJson({ id: request.id, result: { ok: output } }),
          { headers: { "content-type": "application/json" } },
        );
      },
    }),
  );
  const parsed = await client.parseSql("SELECT :id FROM items", {
    scopeId: "work",
    signal,
  });
  assert.ok("select" in parsed);
  if ("select" in parsed) {
    assert.equal(parsed.select.collection, "items");
    assert.equal(parsed.select.distinct, false);
    assert.equal(
      parsed.select.projection?.[9]?.expr &&
        exact(parsed.select.projection[9].expr),
      exact(expr.literal(null)),
    );
  }
  await client.query(parsed, { params: { id: "a" } });
  const invoked = await client.invoke(
    commands.parseSql,
    { query: "SELECT :id FROM items", scope_id: "work" },
    { signal },
  );
  await client.query(invoked, { params: { id: "b" } });
  assert.equal(parseCalls, 2);
  assert.equal(queryCalls, 2);
});

test("generator resolves recursive Named types, entity IDs and every variant tag", () => {
  const node = (kind: TypeNode["kind"]): TypeNode => ({
    kind,
    constraints: [],
    annotations: [],
  });
  const named = node({ named: { name: "Tree", args: [] } });
  const cases = [
    {
      name: "empty",
      payload: "unit" as const,
      discriminant: null,
      meta: emptyMeta(),
    },
    {
      name: "child",
      payload: { newtype: named },
      discriminant: null,
      meta: emptyMeta(),
    },
  ];
  const pkg = new PackageBuilder("variant-test")
    .root((module) => {
      for (const [name, tag] of [
        ["Tree", "externally_tagged"],
        ["Plain", "untagged"],
        ["Internal", { internally_tagged: { field: "kind" } }],
        [
          "Adjacent",
          { adjacently_tagged: { tag_field: "tag", data_field: "data" } },
        ],
      ] as const)
        module.type({
          name,
          module: null,
          params: [],
          ty: node({ variant: { tag, variants: cases } }),
          visibility: "public",
          meta: emptyMeta(),
        });
      module.type({
        name: "EntityId",
        module: null,
        params: [],
        ty: node({ ref: { name: "Tree" } }),
        visibility: "public",
        meta: emptyMeta(),
      });
      module.type({
        name: "Dictionary",
        module: null,
        params: [],
        ty: node({
          record: {
            fields: {},
            open: true,
            additional: named,
            required_order: null,
          },
        }),
        visibility: "public",
        meta: emptyMeta(),
      });
    })
    .build();
  const output = renderPackage(packageModel(pkg));
  assert.match(output, /export type Tree = "empty" \| \{ "child": Tree \}/);
  assert.match(output, /export type Plain = null \| Tree/);
  assert.match(
    output,
    /export type Internal = \{ "kind": "empty" \} \| \{ "kind": "child" \} & \(Tree\)/,
  );
  assert.match(
    output,
    /export type Adjacent = \{ "tag": "empty" \} \| \{ "tag": "child"; "data": Tree \}/,
  );
  assert.match(output, /export type EntityId = string/);
  assert.match(output, /export type Dictionary = \{ \[key: string\]: Tree \}/);
  assert.doesNotMatch(output, /\$variant/);
});
