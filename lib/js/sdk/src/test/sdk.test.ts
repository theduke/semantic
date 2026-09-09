import test from "node:test";
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { createServer } from "node:http";
import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import {
  BatchBuilder,
  FileClient,
  HttpTransport,
  PackageBuilder,
  WebSocketTransport,
  decodeTagged,
  decodeTaggedExact,
  emptyMeta,
  encodeTagged,
  flatJson,
  packageJson,
  parseJson,
  stringifyJson,
  value,
} from "../index.js";
import { packageModel, renderPackage } from "../generator/index.js";
import type {
  Package,
  SemanticObject,
  TypeDef,
  TypeNode,
  WebSocketLike,
} from "../index.js";

test("tagged values preserve unsafe integers exactly", () => {
  const input = { small: 4, huge: 2n ** 100n, rows: [true, "x"] };
  const tagged = encodeTagged(input, "i128");
  const wire = stringifyJson(tagged);
  assert.match(wire, /1267650600228229401496703205376/);
  const decoded = decodeTagged(parseJson(wire) as never) as SemanticObject;
  assert.equal(decoded.small, 4);
  assert.equal(decoded.huge, input.huge);
  assert.deepEqual(decoded.rows, input.rows);
});

test("integer widths validate ranges and typed helpers are not double encoded", () => {
  assert.throws(() => encodeTagged(2n ** 63n), /outside the i64 range/);
  assert.throws(() => value.int("u8", 256), /outside the u8 range/);
  assert.throws(
    () => encodeTagged(Number.MAX_SAFE_INTEGER + 1),
    /safe integer/,
  );
  assert.throws(() => decodeTagged({ u8: 256 }), /outside the u8 range/);
  assert.throws(() => decodeTagged({ bytes: [256] }), /outside the u8 range/);
  assert.deepEqual(encodeTagged(value.int("u8", 7)), { u8: 7n });
  assert.deepEqual(encodeTagged(decodeTaggedExact({ i16: -4 })), { i16: -4 });
  const tagged = encodeTagged({
    id: value.uuid("0f8fad5b-d9cb-469f-a165-70867728950e"),
  }) as { object: Record<string, unknown> };
  assert.deepEqual(tagged.object.id, {
    uuid: "0f8fad5b-d9cb-469f-a165-70867728950e",
  });
});

test("lossless JSON retains unsafe tokens and protects object prototypes", () => {
  const parsed = parseJson(
    '{"n":9007199254740993,"__proto__":{"polluted":true}}',
  ) as Record<string, unknown>;
  assert.equal(parsed.n, 9007199254740993n);
  assert.equal(Object.getPrototypeOf(parsed), null);
  assert.equal(({} as { polluted?: boolean }).polluted, undefined);
  assert.equal(Object.prototype.hasOwnProperty.call(parsed, "__proto__"), true);
});

test("package, facet value, and flat file JSON remain distinct", () => {
  const pkg = new PackageBuilder("example")
    .root((module) =>
      module.type({
        name: "fixture",
        module: null,
        params: [],
        ty: {
          kind: {
            record: {
              fields: {
                item: {
                  ty: { kind: "uuid", constraints: [], annotations: [] },
                  required: false,
                  readonly: false,
                  writeonly: false,
                  default: { literal: 7 },
                  meta: emptyMeta(),
                },
                contains: {
                  ty: { kind: "uuid", constraints: [], annotations: [] },
                  required: false,
                  readonly: false,
                  writeonly: false,
                  default: null,
                  meta: emptyMeta(),
                },
                literal: {
                  ty: { kind: "uuid", constraints: [], annotations: [] },
                  required: false,
                  readonly: false,
                  writeonly: false,
                  default: null,
                  meta: emptyMeta(),
                },
                default_value: {
                  ty: { kind: "uuid", constraints: [], annotations: [] },
                  required: false,
                  readonly: false,
                  writeonly: false,
                  default: null,
                  meta: emptyMeta(),
                },
                discriminant: {
                  ty: { kind: "uuid", constraints: [], annotations: [] },
                  required: false,
                  readonly: false,
                  writeonly: false,
                  default: null,
                  meta: emptyMeta(),
                },
              },
              open: false,
              additional: null,
              required_order: null,
            },
          },
          constraints: [{ default_expr: { expr: { literal: { value: 3 } } } }],
          annotations: [],
        },
        visibility: "public",
        meta: emptyMeta(),
      }),
    )
    .migration("data", "v1", (migration) => {
      migration.insert("things", "one", { count: 9 });
      migration.operation({
        update: {
          query: {
            collection: null,
            predicate: { operand: { literal: { literal: 11 } } },
            assignments: [],
            limit: null,
            returning: [],
            field_format: "plain",
          },
        },
      });
    })
    .build();
  const wire = packageJson.stringify(pkg);
  assert.match(wire, /"count":\{"i64":9\}/);
  assert.match(wire, /"default":\{"object":\{"literal":\{"i64":7\}\}\}/);
  assert.match(
    wire,
    /"default_expr":\{"expr":\{"literal":\{"value":\{"i64":3\}\}\}\}/,
  );
  for (const name of ["contains", "literal", "default_value", "discriminant"])
    assert.match(wire, new RegExp(`"${name}":\\{"ty":\\{"kind":"uuid"`));
  assert.match(
    wire,
    /"operand":\{"literal":\{"object":\{"literal":\{"i64":11\}\}\}\}/,
  );
  assert.deepEqual(
    (
      packageJson.parse(wire).migrations[0]!.operations[0] as {
        insert: { object: SemanticObject };
      }
    ).insert.object.count,
    9,
  );
  assert.equal(flatJson.stringify({ count: 9 }), '{"count":9}');
  assert.equal(
    flatJson.stringify({ count: 9007199254740993n }),
    '{"count":9007199254740993}',
  );
});

test("HTTP transport uses the tagged RPC envelope", async () => {
  let body = "";
  const fetcher: typeof fetch = async (_input, init) => {
    body = String(init?.body);
    return new Response('{"id":1,"result":{"ok":{"u64":9007199254740993}}}', {
      status: 200,
    });
  };
  const result = await new HttpTransport("https://example.test/rpc", {
    fetch: fetcher,
  }).invoke("x", { ok: true });
  assert.equal(result, 9007199254740993n);
  assert.equal((parseJson(body) as { command: string }).command, "x");
});

test("HTTP transport calls the global fetch with the correct receiver", async () => {
  const originalFetch = globalThis.fetch;
  let receiver: unknown;
  globalThis.fetch = async function (this: unknown) {
    receiver = this;
    return new Response('{"id":1,"result":{"ok":"void"}}');
  } as typeof fetch;

  try {
    await new HttpTransport("https://example.test/rpc").invoke("x", {});
    assert.equal(receiver, globalThis);
  } finally {
    globalThis.fetch = originalFetch;
  }
});

test("HTTP transport errors include the endpoint and underlying error", async () => {
  const endpoint = "http://127.0.0.1:3000/api/v1/rpc";
  const fetcher: typeof fetch = async () => {
    throw new TypeError("Failed to fetch");
  };

  await assert.rejects(
    new HttpTransport(endpoint, { fetch: fetcher }).invoke("x", {}),
    new RegExp(
      `HTTP RPC request to ${endpoint.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")} failed: TypeError: Failed to fetch`,
    ),
  );
});

test("HTTP transport status errors include the endpoint and status text", async () => {
  const endpoint = "https://example.test/api/v1/rpc";
  const fetcher: typeof fetch = async () =>
    new Response(null, { status: 503, statusText: "Service Unavailable" });

  await assert.rejects(
    new HttpTransport(endpoint, { fetch: fetcher }).invoke("x", {}),
    /HTTP RPC request to https:\/\/example\.test\/api\/v1\/rpc failed: 503 Service Unavailable/,
  );
});

class FakeSocket implements WebSocketLike {
  readyState: number;
  sent: string[] = [];
  private listeners = new Map<string, Array<(event: any) => void>>();
  constructor(state = 0) {
    this.readyState = state;
  }
  send(data: string): void {
    if (this.readyState !== 1) throw new Error("not open");
    this.sent.push(data);
  }
  close(): void {
    this.readyState = 3;
    this.emit("close", {});
  }
  addEventListener(type: string, listener: (event: any) => void): void {
    const listeners = this.listeners.get(type) ?? [];
    listeners.push(listener);
    this.listeners.set(type, listeners);
  }
  emit(type: string, event: any): void {
    for (const listener of this.listeners.get(type) ?? []) listener(event);
  }
}

test("WebSocket transport handles already-open and close-before-open sockets", async () => {
  const open = new FakeSocket(1);
  const transport = new WebSocketTransport("ws://test", () => open);
  const response = transport.invoke("ping", {});
  await Promise.resolve();
  const envelope = parseJson(open.sent[0]!) as { id: number | bigint };
  open.emit("message", {
    data: stringifyJson({ id: envelope.id, result: { ok: "void" } }),
  });
  assert.equal(await response, undefined);
  transport.close();

  const connecting = new FakeSocket();
  const closed = new WebSocketTransport("ws://test", () => connecting);
  const pending = closed.invoke("ping", {});
  closed.close();
  await assert.rejects(pending, /closed by client/);

  // A consumer is allowed to ignore readiness without causing an unhandled
  // rejection when the transport is closed while connecting.
  const abandoned = new WebSocketTransport("ws://test", () => new FakeSocket());
  abandoned.close();
  await new Promise((resolvePromise) => setImmediate(resolvePromise));
});

test("WebSocket transport explains how to run where WebSocket is unavailable", () => {
  const descriptor = Object.getOwnPropertyDescriptor(globalThis, "WebSocket");
  Object.defineProperty(globalThis, "WebSocket", {
    configurable: true,
    value: undefined,
  });
  try {
    assert.throws(
      () => new WebSocketTransport("ws://test"),
      /provide a WebSocketFactory/,
    );
  } finally {
    if (descriptor) Object.defineProperty(globalThis, "WebSocket", descriptor);
    else Reflect.deleteProperty(globalThis, "WebSocket");
  }
});

test("file ranges handle empty and validated partial reads", async () => {
  let calls = 0;
  const fetcher: typeof fetch = async (_input, init) => {
    calls++;
    assert.equal(new Headers(init?.headers).get("range"), "bytes=2-4");
    return new Response(Uint8Array.from([2, 3, 4]), {
      status: 206,
      headers: { "content-range": "bytes 2-4/10" },
    });
  };
  const files = new FileClient("https://example.test/file", fetcher);
  assert.equal((await files.read("id", { size: 0 })).byteLength, 0);
  assert.equal(calls, 0);
  assert.deepEqual(
    await files.read("id", { offset: 2, size: 3 }),
    Uint8Array.from([2, 3, 4]),
  );
  await assert.rejects(files.read("id", { offset: -1 }), /non-negative/);
  const invalid = new FileClient(
    "https://example.test/file",
    async () =>
      new Response(Uint8Array.from([2, 3, 4]), {
        status: 206,
        headers: { "content-range": "bytes 2-4/4" },
      }),
  );
  await assert.rejects(
    invalid.read("id", { offset: 2, size: 3 }),
    /invalid Content-Range/,
  );
});

test("Node fetch accepts streaming file uploads", async () => {
  let received = "";
  const server = createServer((request, response) => {
    request.setEncoding("utf8");
    request.on("data", (chunk) => (received += chunk));
    request.on("end", () => {
      response.writeHead(200, { "content-type": "application/json" });
      response.end('{"id":"one","collection":"files","object":{}}');
    });
  });
  await new Promise<void>((resolvePromise) =>
    server.listen(0, "127.0.0.1", resolvePromise),
  );
  try {
    const address = server.address();
    assert.ok(address && typeof address === "object");
    const stream = new ReadableStream<Uint8Array>({
      start(controller) {
        controller.enqueue(new TextEncoder().encode("streamed"));
        controller.close();
      },
    });
    const result = await new FileClient(
      `http://127.0.0.1:${address.port}/file`,
    ).upload({ content: stream });
    assert.equal(result.id, "one");
    assert.equal(received, "streamed");
  } finally {
    await new Promise<void>((resolvePromise, reject) =>
      server.close((error) => (error ? reject(error) : resolvePromise())),
    );
  }
});

test("browser fetch receives duplex mode for streaming file uploads", async () => {
  const processDescriptor = Object.getOwnPropertyDescriptor(
    globalThis,
    "process",
  );
  let init: RequestInit | undefined;
  Object.defineProperty(globalThis, "process", {
    configurable: true,
    writable: true,
    value: { versions: {} },
  });
  try {
    const upload = new FileClient(
      "https://example.test/file",
      async (_, value) => {
        init = value;
        return {
          ok: true,
          text: async () => '{"id":"one","collection":"files","object":{}}',
        } as Response;
      },
    ).upload({
      content: new ReadableStream({
        start(controller) {
          controller.close();
        },
      }),
    });
    assert.equal(
      (init as (RequestInit & { duplex?: string }) | undefined)?.duplex,
      "half",
    );
    await upload;
  } finally {
    if (processDescriptor)
      Object.defineProperty(globalThis, "process", processDescriptor);
    else delete (globalThis as { process?: unknown }).process;
  }
});

test("builders deeply snapshot inputs and outputs", () => {
  const entity = { nested: { value: "before" } };
  const builder = new BatchBuilder().upsert("one", entity, "things");
  entity.nested.value = "after";
  const first = builder.build();
  assert.equal(
    (first[0] as unknown as { object: { nested: { value: string } } }).object
      .nested.value,
    "before",
  );
  (
    first[0] as unknown as { object: { nested: { value: string } } }
  ).object.nested.value = "changed";
  assert.equal(
    (builder.build()[0] as unknown as { object: { nested: { value: string } } })
      .object.nested.value,
    "before",
  );
});

const generatorFixture = (): Package => {
  const stringNode: TypeNode = {
    kind: { string: { format: null, normalization: null } },
    constraints: [],
    annotations: [],
  };
  return new PackageBuilder("example")
    .root((module) => {
      module.type({
        name: "user-id",
        module: null,
        params: [],
        ty: stringNode,
        visibility: "public",
        meta: emptyMeta(),
      });
      module.type({
        name: "result",
        module: null,
        params: [],
        ty: {
          kind: {
            result: {
              ok: {
                kind: { ref: { name: "user-id", args: [] } },
                constraints: [],
                annotations: [],
              },
              err: stringNode,
            },
          },
          constraints: [],
          annotations: [],
        },
        visibility: "public",
        meta: emptyMeta(),
      });
      module.type({
        name: "box",
        module: null,
        params: [{ name: "T", bounds: [], default: null }],
        ty: {
          kind: {
            optional: {
              inner: {
                kind: { ref: { name: "T", args: [] } },
                constraints: [],
                annotations: [],
              },
            },
          },
          constraints: [],
          annotations: [],
        },
        visibility: "public",
        meta: emptyMeta(),
      });
      module.type({
        name: "class",
        module: null,
        params: [],
        ty: stringNode,
        visibility: "public",
        meta: emptyMeta(),
      });
      module.type({
        name: "class-holder",
        module: null,
        params: [],
        ty: {
          kind: { ref: { name: "class", args: [] } },
          constraints: [],
          annotations: [],
        },
        visibility: "public",
        meta: emptyMeta(),
      });
      module.type({
        name: "external",
        module: null,
        params: [],
        ty: {
          kind: { ref: { name: "missing", args: [stringNode] } },
          constraints: [],
          annotations: [],
        },
        visibility: "public",
        meta: emptyMeta(),
      });
      module.contract("api", (contract) =>
        contract.command("user.get", {
          params: [{ name: "user-id", ty: stringNode }],
          results: [
            {
              kind: { ref: { name: "result", args: [] } },
              constraints: [],
              annotations: [],
            },
          ],
          throws: null,
          async_fn: true,
        }),
      );
    })
    .build();
};

test("package generator handles nested kinds, references, commands, and symbols", () => {
  const output = renderPackage(packageModel(generatorFixture()));
  assert.match(output, /export type user_id = string/);
  assert.match(output, /\{ ok: user_id \} \| \{ err: string \}/);
  assert.match(output, /export type box<T> = T \| null/);
  assert.match(output, /export type _class = string/);
  assert.match(output, /export type class_holder = _class/);
  assert.match(output, /export type external = SemanticValue/);
  assert.doesNotMatch(output, /SemanticValue</);
  assert.match(output, /export const user_get = command/);
});

const typeDefinition = (name: string, ty: TypeNode): TypeDef => ({
  name,
  module: null,
  params: [],
  ty,
  visibility: "public",
  meta: emptyMeta(),
});

test("package generator uses lexical scopes and includes every declaration owner", () => {
  const stringNode: TypeNode = {
    kind: { string: { format: null, normalization: null } },
    constraints: [],
    annotations: [],
  };
  const numberNode: TypeNode = {
    kind: { number: { int: "i32" } },
    constraints: [],
    annotations: [],
  };
  const ref = (name: string): TypeNode => ({
    kind: { ref: { name, args: [] } },
    constraints: [],
    annotations: [],
  });
  const pkg = new PackageBuilder("scopes")
    .module("a", (module) => module.type(typeDefinition("Thing", stringNode)))
    .module("b", (module) => {
      module.type(typeDefinition("Thing", numberNode));
      module.type(typeDefinition("Local", ref("Thing")));
      module.type(typeDefinition("Qualified", ref("a::Thing")));
      module.contract("api");
    })
    .build();
  const module = pkg.modules.b!;
  module.interfaces.Runner = {
    methods: [
      {
        name: "run",
        signature: {
          params: [],
          results: [ref("Thing")],
          throws: null,
          async_fn: false,
        },
      },
    ],
  };
  module.attributes.slug = {
    id: "b:slug",
    name: "slug",
    ty: stringNode,
    constraints: [],
    meta: emptyMeta(),
  };
  module.classes.Base = {
    id: "b:base",
    name: "Base",
    inherits: null,
    extends: [],
    "semantic:class:strict_schema": true,
    attributes: {},
    constraints: [],
    meta: emptyMeta(),
  };
  module.classes.Child = {
    id: "b:child",
    name: "Child",
    inherits: { id: "b:base" },
    extends: [],
    "semantic:class:strict_schema": true,
    attributes: {
      slug: {
        attribute: { id: "b:slug" },
        required: true,
        ui_order: null,
        computed: null,
        constraints: [],
        meta: emptyMeta(),
      },
    },
    constraints: [],
    meta: emptyMeta(),
  };
  module.types.EntityLink = typeDefinition("EntityLink", {
    kind: { class: module.classes.Child },
    constraints: [],
    annotations: [],
  });
  const contract = module.contracts.api!;
  contract.types.ContractThing = typeDefinition("ContractThing", ref("Thing"));
  contract.functions.lookup = {
    name: "lookup",
    signature: {
      params: [],
      results: [ref("Thing")],
      throws: null,
      async_fn: true,
    },
    meta: emptyMeta(),
  };
  contract.attributes.token = {
    id: "b:api:token",
    name: "token",
    ty: stringNode,
    constraints: [],
    meta: emptyMeta(),
  };
  contract.classes.ContractEntity = {
    id: "b:api:entity",
    name: "ContractEntity",
    inherits: null,
    extends: [{ id: "b:base" }],
    "semantic:class:strict_schema": true,
    attributes: {},
    constraints: [],
    meta: emptyMeta(),
  };
  contract.interfaces.Control = {
    name: "Control",
    interface: { methods: [] },
    meta: emptyMeta(),
  };

  const output = renderPackage(packageModel(pkg));
  assert.match(output, /export type a_Thing = string/);
  assert.match(output, /export type b_Thing = number/);
  assert.match(output, /export type Local = b_Thing/);
  assert.match(output, /export type Qualified = a_Thing/);
  assert.match(output, /export type Runner = \{ "run": \(\) => b_Thing;? \}/);
  assert.match(output, /export type ContractThing = b_Thing/);
  assert.match(output, /export type token = string/);
  assert.match(output, /export type Control = \{/);
  assert.match(output, /export type Child = .*slug.* & \(Base\)/);
  assert.match(output, /export type ContractEntity = .* & \(Base\)/);
  assert.match(output, /export type EntityLink = string/);
  assert.match(output, /export const lookup = command<\{  \}, b_Thing>/);

  pkg.modules.c = structuredClone(pkg.modules.b!);
  pkg.modules.c.name = "c";
  pkg.modules.c.types = {
    Ambiguous: typeDefinition("Ambiguous", ref("Thing")),
  };
  pkg.modules.c.attributes = {};
  pkg.modules.c.classes = {};
  pkg.modules.c.interfaces = {};
  pkg.modules.c.contracts = {};
  assert.throws(() => packageModel(pkg), /ambiguous type reference 'Thing'/);
});

const generatorEdgeFixture = (): Package => {
  const node = (kind: TypeNode["kind"]): TypeNode => ({
    kind,
    constraints: [],
    annotations: [],
  });
  const ref = (name: string): TypeNode => node({ ref: { name, args: [] } });
  const record = (field: string): TypeNode =>
    node({
      record: {
        fields: {
          [field]: {
            ty: node({ string: { format: null, normalization: null } }),
            required: true,
            readonly: false,
            writeonly: false,
            default: null,
            meta: emptyMeta(),
          },
        },
        open: false,
        additional: null,
        required_order: null,
      },
    });
  const pkg = new PackageBuilder("generator-edges")
    .root((module) => {
      module.type(typeDefinition("SemanticValue", record("semantic")));
      module.type(typeDefinition("string", record("text")));
      module.type(typeDefinition("Array", record("array")));
      module.type(typeDefinition("Map", record("map")));
      module.type(typeDefinition("Left", record("left")));
      module.type(typeDefinition("Right", record("right")));
      module.type(typeDefinition("Tail", record("tail")));
      module.type({
        ...typeDefinition("Defaulted", ref("T")),
        params: [{ name: "T", bounds: [], default: ref("string") }],
      });
      module.type(typeDefinition("UsesDefault", ref("Defaulted")));
      module.type(typeDefinition("UsesReserved", ref("SemanticValue")));
      module.type(
        typeDefinition(
          "IntersectionOfUnion",
          node({
            intersection: {
              variants: [
                node({ union: { variants: [ref("Left"), ref("Right")] } }),
                ref("Tail"),
              ],
            },
          }),
        ),
      );
      const functionNode = node({
        function: {
          params: [],
          results: [ref("Left")],
          throws: null,
          async_fn: false,
        },
      });
      module.type(
        typeDefinition(
          "UnionOfIntersection",
          node({
            union: {
              variants: [
                node({
                  intersection: { variants: [ref("Left"), ref("Right")] },
                }),
                ref("Tail"),
              ],
            },
          }),
        ),
      );
      module.type(
        typeDefinition(
          "FunctionOrTail",
          node({ union: { variants: [functionNode, ref("Tail")] } }),
        ),
      );
      module.type(
        typeDefinition(
          "OptionalFunction",
          node({ optional: { inner: functionNode } }),
        ),
      );
      module.type(
        typeDefinition(
          "FunctionAndTail",
          node({ intersection: { variants: [functionNode, ref("Tail")] } }),
        ),
      );
      module.type(
        typeDefinition(
          "WideNumberAndTail",
          node({
            intersection: {
              variants: [node({ number: { int: "i64" } }), ref("Tail")],
            },
          }),
        ),
      );
      module.contract("api", (contract) =>
        contract.command("command", {
          params: [],
          results: [ref("SemanticValue")],
          throws: null,
          async_fn: false,
        }),
      );
    })
    .build();
  return pkg;
};

test("package generator protects names, defaults, and compound precedence", () => {
  const output = renderPackage(packageModel(generatorEdgeFixture()));
  assert.match(output, /export type _SemanticValue =/);
  assert.match(output, /export type _string =/);
  assert.match(output, /export type _Array =/);
  assert.match(output, /export type _Map =/);
  assert.match(output, /export type Defaulted<T = _string> = T/);
  assert.match(output, /export type UsesDefault = Defaulted/);
  assert.match(output, /export type UsesReserved = _SemanticValue/);
  assert.match(
    output,
    /export const _command = command<\{  \}, _SemanticValue>/,
  );
  assert.match(
    output,
    /export type IntersectionOfUnion = \(Left \| Right\) & Tail/,
  );
  assert.match(
    output,
    /export type UnionOfIntersection = \(Left & Right\) \| Tail/,
  );
  assert.match(output, /export type FunctionOrTail = \(\(\) => Left\) \| Tail/);
  assert.match(
    output,
    /export type OptionalFunction = \(\(\) => Left\) \| null/,
  );
  assert.match(output, /export type FunctionAndTail = \(\(\) => Left\) & Tail/);
  assert.match(
    output,
    /export type WideNumberAndTail = \(number \| bigint\) & Tail/,
  );
});

test("generated declarations and public core schema compile for browsers", async () => {
  const directory = await mkdtemp(join(tmpdir(), "semantic-generator-test-"));
  try {
    const generated = join(directory, "generated.ts");
    const edgeGenerated = join(directory, "edge-generated.ts");
    const config = join(directory, "tsconfig.json");
    await writeFile(
      generated,
      `${renderPackage(packageModel(generatorFixture()))}\nimport type { CoreSchema } from "@semantic/sdk";\nimport type { Package as ExactPackage } from "@semantic/sdk/core";\ndeclare const corePackage: CoreSchema.Package;\nconst exactPackage: ExactPackage = corePackage;\nvoid exactPackage;\n`,
    );
    await writeFile(
      edgeGenerated,
      `${renderPackage(packageModel(generatorEdgeFixture()))}\ndeclare const usesDefault: UsesDefault;\nconst defaultValue: _string = usesDefault;\ndeclare const intersection: IntersectionOfUnion;\nconst expectedIntersection: (Left | Right) & Tail = intersection;\nconst functionOrTail: FunctionOrTail = { tail: "ok" };\nconst optionalFunction: OptionalFunction = null;\ndeclare const functionAndTail: FunctionAndTail;\nconst expectedFunction: () => Left = functionAndTail;\nconst expectedTail: Tail = functionAndTail;\nvoid [defaultValue, expectedIntersection, functionOrTail, optionalFunction, expectedFunction, expectedTail];\n`,
    );
    await writeFile(
      config,
      JSON.stringify({
        compilerOptions: {
          target: "ES2022",
          module: "NodeNext",
          moduleResolution: "NodeNext",
          strict: true,
          noEmit: true,
          lib: ["ES2022", "DOM", "DOM.Iterable"],
          types: [],
          baseUrl: resolve(),
          paths: {
            "@semantic/sdk": ["src/index.ts"],
            "@semantic/sdk/core": ["src/generated/core.ts"],
          },
        },
        files: [generated, edgeGenerated],
      }),
    );
    execFileSync(
      process.execPath,
      [resolve("node_modules/typescript/bin/tsc"), "-p", config],
      { stdio: "pipe" },
    );
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});
