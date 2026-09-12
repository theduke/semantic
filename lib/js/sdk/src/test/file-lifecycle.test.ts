import assert from "node:assert/strict";
import test from "node:test";
import { FileClient, RpcError, TransportError } from "../index.js";

test("native deletion shares authentication, scope, credentials and cancellation", async () => {
  const controller = new AbortController();
  let calls = 0;
  const files = new FileClient("https://example.com/files", {
    headers: { authorization: "Bearer secret" },
    credentials: "include",
    fetch: async (url, init) => {
      calls++;
      assert.equal(
        String(url),
        "https://example.com/files/a%2Fb?scope=team%20one",
      );
      assert.equal(init?.method, "DELETE");
      assert.equal(
        new Headers(init?.headers).get("authorization"),
        "Bearer secret",
      );
      assert.equal(init?.credentials, "include");
      assert.ok(init?.signal);
      return new Response(null, { status: 204 });
    },
  });
  await files.delete("a/b", { scopeId: "team one", signal: controller.signal });
  controller.abort();
  await assert.rejects(files.delete("a/b", { signal: controller.signal }), {
    name: "AbortError",
  });
  assert.equal(calls, 1);
});

test("file operations preserve structured conflict errors", async () => {
  for (const operation of ["upload", "delete"] as const) {
    const code =
      operation === "upload" ? "file_already_exists" : "file_referenced";
    const files = new FileClient(
      "/files",
      async () =>
        new Response(
          JSON.stringify({
            code,
            message: "conflict",
            data: { object: { id: { string: "same" } } },
          }),
          { status: 409 },
        ),
    );
    await assert.rejects(
      operation === "upload"
        ? files.upload({ id: "same", content: "bytes" })
        : files.delete("same"),
      (error: unknown) =>
        error instanceof RpcError &&
        error.code === code &&
        (error.data as Record<string, unknown>).id === "same",
    );
  }
});

test("deletion rejects non-204 success and preserves unstructured HTTP errors", async () => {
  for (const status of [200, 500]) {
    const files = new FileClient(
      "/files",
      async () => new Response("unexpected", { status }),
    );
    await assert.rejects(
      files.delete("file"),
      (error: unknown) =>
        error instanceof TransportError && error.status === status,
    );
  }
});
