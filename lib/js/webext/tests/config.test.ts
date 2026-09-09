import assert from "node:assert/strict";
import test from "node:test";

import {
  CONFIG_STORAGE_KEY,
  entityUrl,
  loadConfig,
  normalizeBaseUrl,
  originPermission,
  rpcEndpoint,
} from "../src/config.js";

test("normalizes an application URL while preserving a path prefix", () => {
  const baseUrl = normalizeBaseUrl(" https://example.test/semantic/// ");
  assert.equal(baseUrl, "https://example.test/semantic");
  assert.equal(
    rpcEndpoint({ baseUrl }),
    "https://example.test/semantic/api/v1/rpc",
  );
  assert.equal(originPermission(baseUrl), "https://example.test/*");
  assert.equal(
    entityUrl({ baseUrl }, "an id/with slash"),
    "https://example.test/semantic/entities/an%20id%2Fwith%20slash",
  );
});

test("omits ports from host permissions while preserving the RPC destination", () => {
  for (const [input, permission] of [
    ["http://127.0.0.1:3001", "http://127.0.0.1/*"],
    ["http://localhost:8888", "http://localhost/*"],
    ["https://example.test:8443/semantic", "https://example.test/*"],
    ["http://[::1]:3001", "http://[::1]/*"],
  ] as const) {
    const baseUrl = normalizeBaseUrl(input);
    assert.equal(originPermission(baseUrl), permission);
    assert.equal(rpcEndpoint({ baseUrl }), `${input}/api/v1/rpc`);
  }
  assert.equal(
    originPermission("http://127.0.0.1:3000"),
    originPermission("http://127.0.0.1:3001"),
  );
});

test("rejects unsafe or ambiguous application URLs", () => {
  for (const value of [
    "file:///tmp/semantic",
    "https://user:secret@example.test",
    "https://example.test/?scope=one",
    "https://example.test/#fragment",
    "not a URL",
  ]) {
    assert.throws(() => normalizeBaseUrl(value));
  }
});

test("loads and validates configuration from extension storage", async () => {
  const config = await loadConfig({
    async get(key) {
      assert.equal(key, CONFIG_STORAGE_KEY);
      return { [key]: { baseUrl: "http://localhost:8888/" } };
    },
    async set() {},
  });
  assert.deepEqual(config, { baseUrl: "http://localhost:8888" });
});
