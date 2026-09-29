import test from "node:test";
import assert from "node:assert/strict";
import {
  SemanticClient,
  HttpTransport,
  encodeTagged,
  decodeTagged,
  parseJson,
  stringifyJson,
  commands,
} from "../index.js";
import type { BatchReturn, SemanticObject, TaggedValue } from "../index.js";

test("batch returning preserves default wire shape, typed modes and caller options", async () => {
  const controller = new AbortController();
  const requests: SemanticObject[] = [];
  const client = new SemanticClient(
    new HttpTransport("https://example.test/rpc", {
      fetch: async (_url, init) => {
        assert.ok(init?.signal);
        assert.equal(init.signal.aborted, false);
        const envelope = parseJson(String(init?.body)) as {
          id: number;
          payload: TaggedValue;
        };
        const payload = decodeTagged(envelope.payload) as SemanticObject;
        requests.push(payload);
        const stats = {
          "semantic:db:batch:stats": {
            "semantic:db:batch:upserted": 1,
            "semantic:db:batch:deleted": 0,
            "semantic:db:batch:updated": 0,
          },
        };
        const changes = {
          ...stats,
          "semantic:db:batch:changes": [
            {
              "semantic:db:entity:collection": "default",
              id: "a",
              "semantic:db:entity:kind": "upsert",
            },
          ],
        };
        const reply =
          payload.returning === "stats"
            ? stats
            : payload.returning === "changes"
              ? changes
              : typeof payload.returning === "object"
                ? {
                    ...changes,
                    "semantic:db:batch:rows": [
                      {
                        "semantic:db:entity:collection": "default",
                        id: "a",
                        "semantic:db:entity:object": {},
                      },
                    ],
                  }
                : {
                    ...stats,
                    "semantic:db:batch:dataset": {
                      default: { a: { id: "a" } },
                    },
                  };
        return new Response(
          stringifyJson({
            id: envelope.id,
            result: { ok: encodeTagged(reply) },
          }),
        );
      },
    }),
  );
  const options = { scopeId: "scope", signal: controller.signal };
  assert.ok((await client.batch([], options))["semantic:db:batch:dataset"]);
  assert.equal(Object.hasOwn(requests[0]!, "returning"), false);
  assert.ok(
    (await client.batch([], { ...options, returning: "dataset" }))[
      "semantic:db:batch:dataset"
    ],
  );
  assert.equal(
    (await client.batch([], { ...options, returning: "stats" }))[
      "semantic:db:batch:stats"
    ]["semantic:db:batch:upserted"],
    1,
  );
  assert.equal(
    (await client.batch([], { ...options, returning: "changes" }))[
      "semantic:db:batch:changes"
    ][0]?.id,
    "a",
  );
  assert.equal(
    (
      await client.batch([], {
        ...options,
        returning: { projection: { fields: [] } },
      })
    )["semantic:db:batch:rows"][0]?.id,
    "a",
  );
  for (const request of requests) assert.equal(request.scope_id, "scope");
  assert.equal(
    stringifyJson(requests[4]?.returning),
    '{"projection":{"fields":[]}}',
  );
  for (const request of requests)
    assert.equal(Object.hasOwn(request, "signal"), false);
  // The original descriptor keeps its dataset result type.
  assert.ok(
    (await client.invoke(commands.batch, { operations: [] }, options))[
      "semantic:db:batch:dataset"
    ],
  );
});

// Compile-time contracts: only fields guaranteed by the selected mode are visible.
async function typeContracts(client: SemanticClient, mode: BatchReturn) {
  const stats = await client.batch([], { returning: "stats" });
  stats["semantic:db:batch:stats"];
  // @ts-expect-error stats does not contain a dataset
  stats["semantic:db:batch:dataset"];
  // @ts-expect-error stats does not contain changes
  stats["semantic:db:batch:changes"];
  const changes = await client.batch([], { returning: "changes" });
  changes["semantic:db:batch:changes"];
  // @ts-expect-error changes does not contain projected rows
  changes["semantic:db:batch:rows"];
  const rows = await client.batch([], {
    returning: { projection: { fields: ["name"] } },
  });
  rows["semantic:db:batch:rows"];
  const result = await client.batch([], { returning: mode });
  result["semantic:db:batch:stats"];
  // @ts-expect-error a union mode does not guarantee a dataset
  result["semantic:db:batch:dataset"];
  // @ts-expect-error unknown response mode
  await client.batch([], { returning: "invalid" });
}
void typeContracts;
