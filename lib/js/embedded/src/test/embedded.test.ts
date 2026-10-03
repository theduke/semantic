import assert from "node:assert/strict";
import test from "node:test";
import { encodeTagged, parseJson, stringifyJson } from "@semantic/sdk";
import { EmbeddedError, openEmbedded } from "../index.js";
import {
  PROTOCOL_VERSION,
  setNativeModuleForTest,
  type NativeEmbedded,
} from "../native.js";

function mockNative(overrides: Partial<NativeEmbedded> = {}): NativeEmbedded {
  return {
    async invokeJson(request) {
      const envelope = parseJson(request) as { id: bigint };
      return stringifyJson({
        id: envelope.id,
        result: { ok: encodeTagged(9_007_199_254_740_993n) },
      });
    },
    async uploadFile(_content, _options) {
      return stringifyJson({
        id: "file",
        collection: "semantic.file",
        object: encodeTagged({ size: 3n }),
      });
    },
    async readFile() {
      return new Uint8Array([1, 2, 3]);
    },
    async close() {},
    ...overrides,
  };
}

function install(native: NativeEmbedded): void {
  setNativeModuleForTest({
    protocolVersion: () => PROTOCOL_VERSION,
    open: async () => native,
  });
}

test.afterEach(() => setNativeModuleForTest(undefined));

test("uses lossless RPC wire values and closes idempotently", async () => {
  let closes = 0;
  install(
    mockNative({
      close: async () => {
        closes++;
      },
    }),
  );
  const embedded = await openEmbedded({ dataDir: "." });
  assert.equal(
    await embedded.transport.invoke("example", 1),
    9_007_199_254_740_993n,
  );
  const first = embedded.close();
  const second = embedded.close();
  await Promise.all([first, second, embedded.transport.closed]);
  assert.equal(closes, 1);
  await assert.rejects(
    () => embedded.transport.invoke("example", 1),
    (error: unknown) =>
      error instanceof EmbeddedError && error.code === "EMBEDDED_CLOSED",
  );
});

test("validates options before native loading", async () => {
  await assert.rejects(
    () => openEmbedded({ dataDir: "", maxConcurrentRequests: 0 }),
    /dataDir is required/,
  );
  await assert.rejects(
    () =>
      openEmbedded({
        dataDir: ".",
        blobPassword: "secret",
        blobUri: "fs:///tmp/blob",
      }),
    /only valid with a logfs/,
  );
});

test("file input is snapshotted and buffered limits are enforced", async () => {
  let received: Uint8Array | undefined;
  install(
    mockNative({
      uploadFile: async (content) => {
        received = content;
        return stringifyJson({
          id: "file",
          collection: "semantic.file",
          object: encodeTagged({}),
        });
      },
    }),
  );
  const embedded = await openEmbedded({
    dataDir: ".",
    maxBufferedFileBytes: 3,
  });
  const input = new Uint8Array([1, 2, 3]);
  await embedded.files.upload({ content: input });
  input[0] = 9;
  assert.deepEqual(received, new Uint8Array([1, 2, 3]));
  await assert.rejects(
    () => embedded.files.upload({ content: "four" }),
    /exceeds/,
  );
  await embedded.close();
});

test("parseSql and typed parse commands retain exact AST literals through the embedded transport", async () => {
  const { query, expr, projection, value, commands } = await import(
    "@semantic/sdk"
  );
  const ast = query.select("items", {
    projection: [
      projection.field(expr.literal(value.int("u8", 7)), "small"),
      projection.field(
        expr.literal(value.int("u64", 18446744073709551615n)),
        "wide",
      ),
      projection.field(
        expr.literal({ present_void: value.void(), present_null: null }),
        "object",
      ),
      projection.field(expr.parameter("id"), "parameter"),
    ],
  });
  const taggedAst = encodeTagged(ast);
  let executions = 0;
  install(
    mockNative({
      async invokeJson(request) {
        const { id, command, payload } = parseJson(request) as {
          id: bigint;
          command: string;
          payload: { object: Record<string, unknown> };
        };
        if (command === "semantic.db.query.parse_sql")
          return stringifyJson({ id, result: { ok: taggedAst } });
        assert.equal(command, "semantic.db.query");
        assert.equal(
          stringifyJson(payload.object.query),
          stringifyJson(taggedAst),
        );
        assert.equal(Object.hasOwn(payload.object, "format"), false);
        executions++;
        return stringifyJson({
          id,
          result: { ok: encodeTagged({ kind: "select", rows: [] }) },
        });
      },
    }),
  );
  const embedded = await openEmbedded({ dataDir: "." });
  const parsed = await embedded.client.parseSql("SELECT :id FROM items");
  assert.equal("select" in parsed && parsed.select.collection, "items");
  await embedded.client.query(parsed, { params: { id: "a" } });
  const invoked = await embedded.client.invoke(commands.parseSql, {
    query: "SELECT :id FROM items",
  });
  await embedded.client.query(invoked, { params: { id: "b" } });
  assert.equal(executions, 2);
  await embedded.close();
});
