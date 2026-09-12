import test from "node:test";
import assert from "node:assert/strict";
import {
  SemanticClient,
  HttpTransport,
  value,
  parseJson,
  stringifyJson,
  encodeTagged,
  decodeTaggedExact,
} from "../index.js";
import type { SemanticValue, TaggedValue } from "../index.js";

test("SQL parameters use own properties and retain tagged values over HTTP", async () => {
  const params = Object.assign(Object.create({ inherited: "ignored" }), {
    text: "'; DROP TABLE items --",
    wide: value.int("u64", 18446744073709551615n),
    id: value.uuid("0f8fad5b-d9cb-469f-a165-70867728950e"),
    bytes: new Uint8Array([0, 255]),
  }) as Record<string, SemanticValue>;
  Object.defineProperty(params, "__proto__", {
    value: "own value",
    enumerable: true,
  });
  const client = new SemanticClient(
    new HttpTransport("https://example.test/rpc", {
      fetch: async (_input, init) => {
        const request = parseJson(String(init?.body)) as {
          id: number;
          payload: TaggedValue;
        };
        const payload = decodeTaggedExact(request.payload) as Record<
          string,
          SemanticValue
        >;
        assert.equal(
          stringifyJson(encodeTagged(payload.query!)),
          stringifyJson(
            encodeTagged("SELECT :text, :wide, :id, :bytes FROM items"),
          ),
        );
        const sent = payload.params as Record<string, SemanticValue>;
        assert.equal(Object.hasOwn(sent, "inherited"), false);
        assert.equal(Object.hasOwn(sent, "__proto__"), true);
        assert.equal(
          stringifyJson(encodeTagged(sent.wide!)),
          stringifyJson(encodeTagged(params.wide!)),
        );
        assert.equal(
          stringifyJson(encodeTagged(sent.id!)),
          stringifyJson(encodeTagged(params.id!)),
        );
        assert.equal(
          stringifyJson(encodeTagged(sent.bytes!)),
          stringifyJson(encodeTagged(params.bytes!)),
        );
        return new Response(
          stringifyJson({
            id: request.id,
            result: { ok: encodeTagged({ kind: "select", rows: [] }) },
          }),
          { headers: { "content-type": "application/json" } },
        );
      },
    }),
  );
  const result = await client.sql(
    "SELECT :text, :wide, :id, :bytes FROM items",
    { params },
  );
  assert.equal(result.kind, "select");
  if (result.kind === "select") assert.deepEqual(result.rows, []);
});
