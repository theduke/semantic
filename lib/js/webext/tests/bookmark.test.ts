import assert from "node:assert/strict";
import test from "node:test";

import type {
  BatchOperation,
  BatchOutcome,
  RpcTransport,
  QueryResult,
  SemanticObject,
} from "@semantic/sdk";

import {
  bookmarkLookupSql,
  captureBookmark,
  captureBookmarkSerialized,
  createWebBookmarkId,
  findBookmark,
  normalizeWebsiteUrl,
  type BookmarkClient,
} from "../src/bookmark.js";

const config = { baseUrl: "https://semantic.example/app" };

class FakeClient implements BookmarkClient {
  calls: Array<{ command: string; payload: unknown }> = [];
  batches: Array<readonly BatchOperation[]> = [];
  transport: RpcTransport = {
    invoke: async (command, payload) => {
      this.calls.push({ command, payload });
      return [];
    },
  };
  async batch(operations: readonly BatchOperation[]): Promise<BatchOutcome> {
    this.batches.push(operations);
    return {
      dataset: {},
      stats: { upserted: operations.length, updated: 0, deleted: 0 },
    };
  }
  queries: string[] = [];
  inserts: Array<{ id: string; object: object; collection?: string }> = [];

  constructor(private readonly rows: object[]) {}

  async sql<T extends object = SemanticObject>(
    query: string,
  ): Promise<QueryResult<T>> {
    this.queries.push(query);
    return { kind: "select", rows: this.rows as T[] };
  }

  async insert<T extends object>(
    id: string,
    object: T,
    options?: { collection?: string },
  ): Promise<void> {
    this.inserts.push({
      id,
      object,
      ...(options?.collection ? { collection: options.collection } : {}),
    });
  }
}

test("lookup disambiguates by class and exact escaped URL", () => {
  assert.equal(
    bookmarkLookupSql("https://example.test/it's?q='yes'"),
    "SELECT e.id AS id, e.title AS title FROM entities AS e WHERE e.type IN ('semantic:base:web_bookmark') AND (e.url = 'https://example.test/it''s?q=%27yes%27' OR e.\"semantic:url\" = 'https://example.test/it''s?q=%27yes%27') ORDER BY e.id ASC LIMIT 1",
  );
});

test("rejects unsupported or credential-bearing website URLs", () => {
  for (const url of [
    "about:config",
    "file:///tmp/private.html",
    "https://user:secret@example.test",
  ]) {
    assert.throws(() => normalizeWebsiteUrl(url));
  }
});

test("creates a prefixed short UUID for new bookmarks", () => {
  assert.equal(
    createWebBookmarkId("123e4567-e89b-12d3-a456-426614174000"),
    "webbookmark-123e4567e89b",
  );
});

test("returns an existing bookmark and does not insert", async () => {
  const client = new FakeClient([{ id: "saved-1", title: "Saved" }]);
  const result = await captureBookmark(
    client,
    config,
    { url: "https://example.test", title: "Changed" },
    () => "new-1",
  );
  assert.equal(result.created, false);
  assert.equal(
    result.bookmark.href,
    "https://semantic.example/app/entities/saved-1",
  );
  assert.equal(client.inserts.length, 0);
});

test("creates a typed WebBookmark with optional description", async () => {
  const client = new FakeClient([]);
  const result = await captureBookmark(
    client,
    config,
    {
      url: "https://example.test/page",
      title: " Page title ",
      description: " Notes ",
    },
    () => "new-1",
  );
  assert.equal(result.created, true);
  assert.deepEqual(client.inserts, [
    {
      id: "new-1",
      collection: "entities",
      object: {
        id: "new-1",
        type: "semantic:base:web_bookmark",
        url: "https://example.test/page",
        title: "Page title",
        description: "Notes",
      },
    },
  ]);
});

test("uses the hostname when the submitted title is blank", async () => {
  const client = new FakeClient([]);
  const result = await captureBookmark(
    client,
    config,
    { url: "https://www.example.test/path", title: "  " },
    () => "new-2",
  );
  assert.equal(result.bookmark.title, "www.example.test");
});

test("rejects malformed select rows", async () => {
  const client = new FakeClient([{ id: 42 }]);
  await assert.rejects(() =>
    findBookmark(client, config, "https://example.test"),
  );
});

test("serializes concurrent captures for the same server and URL", async () => {
  const client = new FakeClient([]);
  let releaseLookup: (() => void) | undefined;
  const originalSql = client.sql.bind(client);
  client.sql = async <T extends object = SemanticObject>(query: string) => {
    await new Promise<void>((resolve) => {
      releaseLookup = resolve;
    });
    return originalSql<T>(query);
  };
  const factory = () => client;
  const input = { url: "https://example.test/concurrent", title: "Concurrent" };
  const first = captureBookmarkSerialized(factory, config, input);
  const second = captureBookmarkSerialized(factory, config, input);
  assert.equal(first, second);
  releaseLookup?.();
  const [left, right] = await Promise.all([first, second]);
  assert.deepEqual(left, right);
  assert.equal(client.queries.length, 1);
  assert.equal(client.inserts.length, 1);
});

test("saves parent metadata and applies deduplicated labels using the label service", async () => {
  const client = new FakeClient([]);
  await captureBookmark(
    client,
    config,
    {
      url: "https://example.test",
      title: "Example",
      parentId: "project-1",
      labelIds: ["read", "read"],
    },
    () => "new-1",
  );
  assert.equal(
    (client.inserts[0]?.object as SemanticObject)["semantic:parent"],
    "project-1",
  );
  assert.deepEqual(client.calls, [
    {
      command: "semantic.base.labels.replace",
      payload: { id: "new-1", collection: "entities", label_ids: ["read"] },
    },
  ]);
});

test("creates the bookmark and directory membership in one ordered batch", async () => {
  const client = new FakeClient([]);
  client.sql = async <T extends object>(
    query: string,
  ): Promise<QueryResult<T>> => {
    client.queries.push(query);
    const rows = query.includes("MAX(")
      ? [{ next_order: 8n }]
      : query.includes("SELECT id FROM")
        ? [{ id: "folder-1" }]
        : [];
    return { kind: "select", rows: rows as T[] };
  };
  await captureBookmark(
    client,
    config,
    { url: "https://example.test", title: "Example", directoryId: "folder-1" },
    () => "new-1",
  );
  assert.equal(client.inserts.length, 0);
  const batch = client.batches[0]!;
  assert.equal(batch.length, 2);
  assert.equal(batch[0]?.kind, "upsert");
  if (batch[0]?.kind !== "upsert") throw new Error("Expected bookmark upsert");
  assert.equal(batch[0].id, "new-1");
  assert.equal(batch[1]?.kind, "upsert");
  if (batch[1]?.kind !== "upsert")
    throw new Error("Expected membership upsert");
  assert.equal(
    batch[1].object["semantic:base:directory_node:from"],
    "folder-1",
  );
  assert.equal(batch[1].object["semantic:relation:to"], "new-1");
  assert.equal(batch[1].object["semantic:base:directory_node:order"], 9n);
});

test("a removed folder prevents capture before any write", async () => {
  const client = new FakeClient([]);
  await assert.rejects(
    captureBookmark(client, config, {
      url: "https://example.test",
      title: "Example",
      directoryId: "missing",
    }),
    /no longer exists/,
  );
  assert.equal(client.inserts.length + client.batches.length, 0);
});

test("reports label failure as a partial success with a link to the saved bookmark", async () => {
  const client = new FakeClient([]);
  client.transport.invoke = async () => {
    throw new Error("Offline");
  };
  const result = await captureBookmark(
    client,
    config,
    { url: "https://example.test", title: "Example", labelIds: ["read"] },
    () => "new-1",
  );
  assert.equal(result.created, true);
  assert.match(result.warning!, /labels could not be applied/);
  assert.equal(result.bookmark.id, "new-1");
});

test("duplicate capture leaves existing metadata untouched", async () => {
  const client = new FakeClient([{ id: "saved-1" }]);
  const result = await captureBookmark(client, config, {
    url: "https://example.test",
    title: "Example",
    parentId: "other",
    directoryId: "folder",
    labelIds: ["read"],
  });
  assert.equal(result.created, false);
  assert.equal(
    client.inserts.length + client.batches.length + client.calls.length,
    0,
  );
});
