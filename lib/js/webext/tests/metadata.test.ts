import assert from "node:assert/strict";
import test from "node:test";
import {
  labelOptions,
  metadataSearchSql,
  selectLabel,
} from "../src/metadata.js";

test("label options show hierarchy, omit groups, and preserve exclusive group rules", () => {
  const options = labelOptions([
    {
      id: "status",
      type: "semantic:base:label_group",
      "semantic:base:label:name": "Status",
      "semantic:base:label:selection_mode": "exclusive",
    },
    {
      id: "read",
      type: "semantic:base:label",
      "semantic:base:label:name": "Read",
      "semantic:parent": "status",
      "semantic:base:label:color": "#44aa66",
    },
    {
      id: "later",
      type: "semantic:base:label",
      "semantic:base:label:name": "Later",
      "semantic:parent": "status",
    },
    {
      id: "design",
      type: "semantic:base:label",
      "semantic:base:label:name": "Design",
    },
  ]);
  assert.equal(options.length, 3);
  const read = options.find((item) => item.id === "read")!;
  const later = options.find((item) => item.id === "later")!;
  const design = options.find((item) => item.id === "design")!;
  assert.equal(read.detail, "Status · Choose one");
  assert.equal(read.color, "#44aa66");
  assert.deepEqual(
    selectLabel([read, design], later).map((item) => item.id),
    ["design", "later"],
  );
});

test("label paths tolerate cycles and reject non-hex colors", () => {
  const options = labelOptions([
    {
      id: "a",
      type: "semantic:base:label",
      "semantic:parent": "a",
      "semantic:base:label:color": "url(https://example.test)",
    },
  ]);
  assert.equal(options[0]?.detail, "");
  assert.equal(options[0]?.color, undefined);
});

test("metadata search escapes quotes and limits directory results by class", () => {
  const query = metadataSearchSql("directory", "Jane's");
  assert.match(query, /Jane''s/);
  assert.match(query, /type IN \('semantic:base:directory'\)/);
  assert.match(query, /LIMIT 30$/);
  assert.doesNotMatch(metadataSearchSql("parent", "test"), /type IN/);
});
