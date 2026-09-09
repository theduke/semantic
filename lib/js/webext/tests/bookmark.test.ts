import assert from "node:assert/strict";
import test from "node:test";

import type { QueryResult, SemanticObject } from "@semantic/sdk";

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
