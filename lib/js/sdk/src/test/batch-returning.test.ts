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
        const stats = { upserted: 1, deleted: 0, updated: 0 };
        const changes = [{ collection: "default", id: "a", kind: "upsert" }];
        const reply =
          payload.returning === "stats"
            ? { stats }
            : payload.returning === "changes"
              ? { stats, changes }
              : typeof payload.returning === "object"
                ? {
                    stats,
                    changes,
                    rows: [{ collection: "default", id: "a", object: {} }],
                  }
                : { stats, dataset: { default: { a: { id: "a" } } } };
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
  assert.ok((await client.batch([], options)).dataset);
  assert.equal(Object.hasOwn(requests[0]!, "returning"), false);
  assert.ok(
    (await client.batch([], { ...options, returning: "dataset" })).dataset,
  );
  assert.equal(
    (await client.batch([], { ...options, returning: "stats" })).stats.upserted,
    1,
  );
  assert.equal(
    (await client.batch([], { ...options, returning: "changes" })).changes[0]
      ?.id,
    "a",
  );
  assert.equal(
    (
      await client.batch([], {
        ...options,
        returning: { projection: { fields: [] } },
      })
    ).rows[0]?.id,
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
    (await client.invoke(commands.batch, { operations: [] }, options)).dataset,
  );
});

// Compile-time contracts: only fields guaranteed by the selected mode are visible.
async function typeContracts(client: SemanticClient, mode: BatchReturn) {
  const stats = await client.batch([], { returning: "stats" });
  stats.stats;
  // @ts-expect-error stats does not contain a dataset
  stats.dataset;
  // @ts-expect-error stats does not contain changes
  stats.changes;
  const changes = await client.batch([], { returning: "changes" });
  changes.changes;
  // @ts-expect-error changes does not contain projected rows
  changes.rows;
  const rows = await client.batch([], {
    returning: { projection: { fields: ["name"] } },
  });
  rows.rows;
  const result = await client.batch([], { returning: mode });
  result.stats;
  // @ts-expect-error a union mode does not guarantee a dataset
  result.dataset;
  // @ts-expect-error unknown response mode
  await client.batch([], { returning: "invalid" });
}
void typeContracts;
