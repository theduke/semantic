import assert from "node:assert/strict";
import test from "node:test";

import { parseBackgroundRequest } from "../src/messages.js";

test("accepts only the extension's narrow request shapes", () => {
  assert.deepEqual(
    parseBackgroundRequest({ type: "config.get", ignored: true }),
    {
      type: "config.get",
    },
  );
  assert.deepEqual(
    parseBackgroundRequest({
      type: "bookmark.capture",
      bookmark: {
        url: "https://example.test",
        title: "Example",
        description: "Note",
      },
    }),
    {
      type: "bookmark.capture",
      bookmark: {
        url: "https://example.test",
        title: "Example",
        description: "Note",
      },
    },
  );
  assert.equal(
    parseBackgroundRequest({ type: "bookmark.lookup", url: 42 }),
    null,
  );
  assert.equal(
    parseBackgroundRequest({ type: "bookmark.capture", bookmark: {} }),
    null,
  );
  assert.equal(parseBackgroundRequest({ type: "unknown" }), null);
});

test("metadata messages preserve only valid selection fields", () => {
  const bookmark = {
    url: "https://example.test",
    title: "Example",
    directoryId: "folder",
    parentId: "project",
    labelIds: ["read"],
  };
  assert.deepEqual(
    parseBackgroundRequest({ type: "bookmark.capture", bookmark }),
    { type: "bookmark.capture", bookmark },
  );
  for (const invalid of [
    { parentId: 3 },
    { directoryId: " " },
    { labelIds: [3] },
    { labelIds: "read" },
  ]) {
    assert.equal(
      parseBackgroundRequest({
        type: "bookmark.capture",
        bookmark: { ...bookmark, ...invalid },
      }),
      null,
    );
  }
  assert.deepEqual(
    parseBackgroundRequest({
      type: "metadata.search",
      kind: "labels",
      query: "read",
    }),
    { type: "metadata.search", kind: "labels", query: "read" },
  );
  assert.equal(
    parseBackgroundRequest({
      type: "metadata.search",
      kind: "sql",
      query: "SELECT *",
    }),
    null,
  );
});
