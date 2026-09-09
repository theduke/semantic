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
