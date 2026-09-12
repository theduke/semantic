import assert from "node:assert/strict";
import { createServer } from "node:http";
import { getEventListeners } from "node:events";
import test from "node:test";
import {
  createHttpClient,
  FileClient,
  HttpTransport,
  SemanticClient,
  TransportError,
  WebSocketTransport,
  type WebSocketLike,
  type SemanticValue,
} from "../index.js";

const tick = () => new Promise<void>((resolve) => setImmediate(resolve));
const isAbort = (error: unknown) =>
  error instanceof Error && error.name === "AbortError";
const rpcReply = () => new Response('{"id":1,"result":{"ok":{"object":{}}}}');

test("shared HTTP options reach RPC, upload and download with protocol header precedence", async () => {
  const calls: { url: string; init: RequestInit }[] = [];
  const base = new Headers({
    authorization: "Bearer token",
    "content-type": "base/type",
    range: "bytes=99-",
  });
  const { client, files } = createHttpClient(
    "https://example.test/custom/rpc",
    {
      headers: base,
      credentials: "include",
      fileEndpoint: "https://example.test/blobs",
      fetch: async (url, init) => {
        calls.push({ url: String(url), init: init! });
        if (String(url).endsWith("rpc")) return rpcReply();
        if (init?.method === "POST")
          return new Response('{"id":"f","collection":"files","object":{}}');
        return new Response("abc", { headers: { "content-length": "3" } });
      },
    },
  );
  await client.sql("SELECT 1");
  await files.upload({ content: "abc", mimeType: "text/plain", scopeId: "s" });
  assert.equal(
    new TextDecoder().decode(await files.read("f", { scopeId: "s" })),
    "abc",
  );
  assert.deepEqual(
    calls.map((c) => c.url),
    [
      "https://example.test/custom/rpc",
      "https://example.test/blobs?scope=s",
      "https://example.test/blobs/f?scope=s",
    ],
  );
  for (const { init } of calls) {
    assert.equal(
      new Headers(init.headers).get("authorization"),
      "Bearer token",
    );
    assert.equal(init.credentials, "include");
  }
  assert.equal(
    new Headers(calls[0]!.init.headers).get("content-type"),
    "application/json",
  );
  assert.equal(
    new Headers(calls[1]!.init.headers).get("content-type"),
    "text/plain",
  );
  assert.equal(new Headers(calls[2]!.init.headers).has("range"), false);
  assert.equal(base.get("content-type"), "base/type");
  assert.equal(createHttpClient("/custom/rpc").files.endpoint, "/custom/file");
});

test("all client operations forward request signals outside the wire payload", async () => {
  const signal = new AbortController().signal;
  const seen: string[] = [];
  const client = new SemanticClient({
    async invoke(command, payload, options): Promise<SemanticValue> {
      assert.equal(options?.signal, signal);
      assert.equal(Object.hasOwn(payload as object, "signal"), false);
      seen.push(command);
      return command.endsWith("package.upsert") ? { outcome: "{}" } : {};
    },
  });
  await client.invoke({ name: "custom" }, {}, { signal });
  await client.sql("SELECT 1", { signal });
  await client.prql("from default", { signal });
  await client.get("x", { signal });
  await client.insert("x", {}, { signal });
  await client.delete("x", { signal });
  await client.batch([], { signal });
  assert.equal(seen.length, 7);
});

for (const source of ["connection", "request"] as const) {
  test(`HTTP cancellation via ${source} rejects RPC and upload as AbortError`, async () => {
    for (const operation of ["rpc", "upload"] as const) {
      const connection = new AbortController();
      const request = new AbortController();
      let received: AbortSignal | null | undefined;
      const { client, files } = createHttpClient("/rpc", {
        signal: connection.signal,
        fetch: async (_, init) => {
          received = init?.signal;
          return new Promise<Response>(() => {});
        },
      });
      const pending =
        operation === "rpc"
          ? client.sql("SELECT 1", { signal: request.signal })
          : files.upload({ content: "x", signal: request.signal });
      (source === "connection" ? connection : request).abort(
        new Error("consumer stopped"),
      );
      await assert.rejects(pending, isAbort);
      assert.equal(received?.aborted, true);
      assert.equal(
        (source === "connection" ? request : connection).signal.aborted,
        false,
      );
    }
  });
}

test("pre-aborted operations avoid fetch; RPC response body cancellation remains AbortError", async () => {
  let calls = 0;
  const controller = new AbortController();
  controller.abort("stop");
  const { client, files } = createHttpClient("/rpc", {
    fetch: async () => {
      calls++;
      return rpcReply();
    },
  });
  await assert.rejects(
    client.sql("SELECT 1", { signal: controller.signal }),
    isAbort,
  );
  await assert.rejects(
    files.upload({ content: "", signal: controller.signal }),
    isAbort,
  );
  await assert.rejects(
    files.readStream("x", { signal: controller.signal }),
    isAbort,
  );
  assert.equal(calls, 0);
  const bodyAbort = new AbortController();
  const response = new Response(
    new ReadableStream({
      pull() {
        bodyAbort.abort();
      },
    }),
  );
  await assert.rejects(
    new HttpTransport("/rpc", { fetch: async () => response }).invoke(
      "x",
      {},
      { signal: bodyAbort.signal },
    ),
    isAbort,
  );
});

test("stream metadata and buffered reads preserve clipped and open-ended range semantics", async () => {
  const files = new FileClient("/file", async (_, init) => {
    assert.equal(new Headers(init?.headers).get("range"), "bytes=2-9");
    return new Response(Uint8Array.of(2, 3, 4), {
      status: 206,
      headers: {
        "content-range": "bytes 2-4/5",
        "content-length": "3",
        "content-type": "test/data",
      },
    });
  });
  const result = await files.readStream("x", { offset: 2, size: 8 });
  assert.deepEqual(
    { ...result, stream: undefined },
    {
      stream: undefined,
      status: 206,
      contentRange: "bytes 2-4/5",
      contentLength: 3,
      contentType: "test/data",
      totalSize: 5,
    },
  );
  await result.stream.cancel();
  assert.deepEqual(
    await files.read("x", { offset: 2, size: 8 }),
    Uint8Array.of(2, 3, 4),
  );
  const open = new FileClient("/file", async (_, init) => {
    assert.equal(new Headers(init?.headers).get("range"), "bytes=2-");
    return new Response(Uint8Array.of(2), {
      status: 206,
      headers: { "content-range": "bytes 2-2/*" },
    });
  });
  assert.deepEqual(await open.read("x", { offset: 2 }), Uint8Array.of(2));
});

test("zero-length and unsatisfiable reads return empty streams and release response bodies", async () => {
  let calls = 0;
  let canceled = false;
  const body = new ReadableStream<Uint8Array>({
    cancel() {
      canceled = true;
    },
  });
  const files = new FileClient("/file", async () => {
    calls++;
    return new Response(body, {
      status: 416,
      headers: { "content-range": "bytes */7" },
    });
  });
  assert.equal((await files.read("x", { size: 0 })).length, 0);
  assert.equal(calls, 0);
  const result = await files.readStream("x", { offset: 10 });
  assert.equal(result.status, 416);
  assert.equal(result.totalSize, 7);
  assert.equal((await result.stream.getReader().read()).done, true);
  assert.equal(canceled, true);
  assert.equal(body.locked, false);
});

test("invalid caller ranges reject before issuing requests", async () => {
  const files = new FileClient("/file", async () => {
    throw new Error("unexpected fetch");
  });
  for (const options of [
    { offset: -1 },
    { offset: 0.5 },
    { offset: NaN },
    { size: -1 },
    { size: Infinity },
    { offset: Number.MAX_SAFE_INTEGER, size: 2 },
  ]) {
    await assert.rejects(files.readStream("x", options), RangeError);
  }
});

test("malformed response headers are rejected before exposing bytes and cancel the body", async () => {
  const cases: [number, Record<string, string>, boolean][] = [
    [200, {}, true],
    [206, {}, true],
    [206, { "content-range": "bytes 1-3/9" }, true],
    [206, { "content-range": "bytes 2-5/9" }, true],
    [206, { "content-range": "bytes 2-4/4" }, true],
    [206, { "content-range": "bytes 2-4/9007199254740992" }, true],
    [206, { "content-range": "bytes 2-4/9", "content-length": "4" }, true],
    [200, { "content-length": "-1" }, false],
    [200, { "content-length": "3.5" }, false],
    [200, { "content-length": "9007199254740992" }, false],
    [206, { "content-range": "bytes 0-1/2" }, false],
    [416, { "content-range": "bytes */oops" }, true],
  ];
  for (const [status, headers, ranged] of cases) {
    let canceled = false;
    const body = new ReadableStream<Uint8Array>({
      cancel() {
        canceled = true;
      },
    });
    const files = new FileClient(
      "/file",
      async () => new Response(body, { status, headers }),
    );
    await assert.rejects(
      files.readStream("x", ranged ? { offset: 2, size: 3 } : {}),
      TransportError,
    );
    assert.equal(canceled, true, JSON.stringify(headers));
    assert.equal(body.locked, false);
  }
});

test("truncated and oversized bodies fail with TransportError for full and range reads", async () => {
  for (const ranged of [false, true]) {
    for (const size of [2, 4]) {
      const body = new ReadableStream<Uint8Array>({
        start(c) {
          c.enqueue(new Uint8Array(size));
          c.close();
        },
      });
      const headers: Record<string, string> = { "content-length": "3" };
      if (ranged) headers["content-range"] = "bytes 2-4/9";
      const files = new FileClient(
        "/file",
        async () => new Response(body, { status: ranged ? 206 : 200, headers }),
      );
      await assert.rejects(
        files.read("x", ranged ? { offset: 2, size: 3 } : {}),
        TransportError,
      );
      await tick();
      assert.equal(body.locked, false);
    }
  }
});

test("1-GiB fixture with 64-KiB chunks keeps at most two prefetched chunks", async () => {
  const chunk = new Uint8Array(64 * 1024);
  const totalChunks = (1024 * 1024 * 1024) / chunk.byteLength;
  let produced = 0;
  let consumed = 0;
  const body = new ReadableStream<Uint8Array>({
    pull(controller) {
      if (produced === totalChunks) controller.close();
      else {
        produced++;
        controller.enqueue(chunk);
      }
      assert.ok(
        produced - consumed <= 2,
        `prefetched ${produced - consumed} chunks`,
      );
    },
  });
  const files = new FileClient(
    "/file",
    async () =>
      new Response(body, {
        headers: { "content-length": String(totalChunks * chunk.byteLength) },
      }),
  );
  const { stream } = await files.readStream("large");
  await tick();
  assert.ok(produced <= 2);
  const reader = stream.getReader();
  let bytes = 0;
  for (;;) {
    const next = await reader.read();
    if (next.done) break;
    consumed++;
    bytes += next.value.byteLength;
    if (consumed === 1) {
      assert.ok(produced < totalChunks);
      await tick();
      assert.ok(produced - consumed <= 2);
    }
  }
  reader.releaseLock();
  assert.equal(bytes, 1024 * 1024 * 1024);
  assert.equal(body.locked, false);
});

for (const mode of ["reader", "request", "connection"] as const) {
  test(`${mode} cancellation cancels and unlocks a blocked source`, async () => {
    const connection = new AbortController();
    const request = new AbortController();
    let canceled = false;
    const body = new ReadableStream<Uint8Array>({
      cancel() {
        canceled = true;
      },
    });
    const files = new FileClient("/file", {
      signal: connection.signal,
      fetch: async () => new Response(body),
    });
    const { stream } = await files.readStream("x", { signal: request.signal });
    const reader = stream.getReader();
    const pending = reader.read();
    if (mode === "reader") {
      await reader.cancel("done");
      assert.equal((await pending).done, true);
    } else {
      (mode === "request" ? request : connection).abort();
      await assert.rejects(pending, isAbort);
    }
    await tick();
    assert.equal(canceled, true);
    assert.equal(body.locked, false);
    reader.releaseLock();
  });
}

test("network body failures surface as TransportError and release the body lock", async () => {
  const body = new ReadableStream<Uint8Array>({
    pull(c) {
      c.error(new TypeError("disconnected"));
    },
  });
  const files = new FileClient("/file", async () => new Response(body));
  await assert.rejects(files.read("x"), TransportError);
  await tick();
  assert.equal(body.locked, false);
});

test("late custom-fetch responses are canceled after the caller aborts", async () => {
  let respond!: (response: Response) => void;
  let canceled = false;
  const controller = new AbortController();
  const files = new FileClient(
    "/file",
    async () =>
      new Promise<Response>((resolve) => {
        respond = resolve;
      }),
  );
  const pending = files.readStream("x", { signal: controller.signal });
  controller.abort();
  await assert.rejects(pending, isAbort);
  respond(
    new Response(
      new ReadableStream({
        cancel() {
          canceled = true;
        },
      }),
    ),
  );
  await tick();
  assert.equal(canceled, true);
});

test("completed RPC and stream requests release connection and request abort listeners", async () => {
  const connection = new AbortController();
  const request = new AbortController();
  const { client, files } = createHttpClient("/rpc", {
    signal: connection.signal,
    fetch: async (url) =>
      String(url) === "/rpc" ? rpcReply() : new Response("abc"),
  });
  await client.sql("SELECT 1", { signal: request.signal });
  await files.read("x", { signal: request.signal });
  assert.equal(getEventListeners(connection.signal, "abort").length, 0);
  assert.equal(getEventListeners(request.signal, "abort").length, 0);
});

test(
  "real Node download cancellation closes the HTTP response",
  { timeout: 5000 },
  async () => {
    let closed!: () => void;
    const connectionClosed = new Promise<void>((resolve) => {
      closed = resolve;
    });
    const server = createServer((_, response) => {
      response.once("close", closed);
      response.writeHead(200, { "content-length": "100" });
      response.write("abc");
    });
    await new Promise<void>((resolve) =>
      server.listen(0, "127.0.0.1", resolve),
    );
    try {
      const address = server.address();
      assert.ok(address && typeof address === "object");
      const controller = new AbortController();
      const files = new FileClient(`http://127.0.0.1:${address.port}/file`);
      const { stream } = await files.readStream("x", {
        signal: controller.signal,
      });
      const reader = stream.getReader();
      assert.equal(
        new TextDecoder().decode((await reader.read()).value),
        "abc",
      );
      const pending = reader.read();
      controller.abort();
      await assert.rejects(pending, isAbort);
      await connectionClosed;
      reader.releaseLock();
    } finally {
      server.closeAllConnections();
      await new Promise<void>((resolve) => server.close(() => resolve()));
    }
  },
);

test(
  "real Node HTTP authenticates RPC/upload/download and yields the first chunk before EOF",
  { timeout: 5000 },
  async () => {
    let finishDownload: () => void = () => {};
    const received: string[] = [];
    const server = createServer((request, response) => {
      if (request.headers.authorization !== "Bearer shared") {
        response.writeHead(401).end();
        return;
      }
      received.push(request.url!);
      if (request.url === "/rpc") {
        request.resume();
        response.end('{"id":1,"result":{"ok":{"object":{}}}}');
      } else if (request.method === "POST") {
        request.resume();
        request.on("end", () =>
          response.end('{"id":"f","collection":"files","object":{}}'),
        );
      } else {
        response.writeHead(200, {
          "content-type": "application/octet-stream",
          "content-length": "6",
        });
        response.write("abc");
        finishDownload = () => response.end("def");
      }
    });
    await new Promise<void>((resolve) =>
      server.listen(0, "127.0.0.1", resolve),
    );
    try {
      const address = server.address();
      assert.ok(address && typeof address === "object");
      const { client, files } = createHttpClient(
        `http://127.0.0.1:${address.port}/rpc`,
        { headers: { authorization: "Bearer shared" } },
      );
      await client.sql("SELECT 1");
      await files.upload({ content: "abcdef" });
      const result = await files.readStream("f");
      const reader = result.stream.getReader();
      assert.equal(
        new TextDecoder().decode((await reader.read()).value),
        "abc",
      );
      finishDownload();
      assert.equal(
        new TextDecoder().decode((await reader.read()).value),
        "def",
      );
      assert.equal((await reader.read()).done, true);
      reader.releaseLock();
      assert.deepEqual(received, ["/rpc", "/file", "/file/f"]);
    } finally {
      server.closeAllConnections();
      await new Promise<void>((resolve) => server.close(() => resolve()));
    }
  },
);

class Socket implements WebSocketLike {
  readyState = 1;
  sent: string[] = [];
  listeners = new Map<string, ((event: any) => void)[]>();
  send(data: string) {
    this.sent.push(data);
  }
  close() {
    this.readyState = 3;
  }
  addEventListener(type: string, listener: (event: any) => void) {
    this.listeners.set(type, [...(this.listeners.get(type) ?? []), listener]);
  }
  emit(type: string, event: unknown = {}) {
    for (const listener of this.listeners.get(type) ?? []) listener(event);
  }
}

test("WebSocket cancellation removes only the local request and ignores its late response", async () => {
  const socket = new Socket();
  const transport = new WebSocketTransport("ws://test", () => socket);
  const abort = new AbortController();
  const canceled = transport.invoke("one", {}, { signal: abort.signal });
  const active = transport.invoke("two", {});
  await tick();
  abort.abort();
  await assert.rejects(canceled, isAbort);
  assert.equal(socket.readyState, 1);
  assert.equal(socket.sent.length, 2);
  for (const sent of socket.sent) {
    const { id } = JSON.parse(sent) as { id: number };
    socket.emit("message", {
      data: JSON.stringify({ id, result: { ok: { string: "ok" } } }),
    });
  }
  assert.equal(await active, "ok");
  transport.close();
});

test("WebSocket requests can be canceled while connecting without closing the socket", async () => {
  const socket = new Socket();
  socket.readyState = 0;
  const transport = new WebSocketTransport("ws://test", () => socket);
  const controller = new AbortController();
  const pending = transport.invoke("x", {}, { signal: controller.signal });
  controller.abort();
  await assert.rejects(pending, isAbort);
  socket.readyState = 1;
  socket.emit("open");
  await transport.ready;
  assert.equal(socket.sent.length, 0);
  transport.close();
});
