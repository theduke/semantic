import test from "node:test";
import assert from "node:assert/strict";
import { decodeTagged, encodeTagged, stringifyJson, value } from "../index.js";
import type {
  JobRecord,
  JobListPage,
  JobListParams,
  JobsCommands,
} from "../generated/core.js";

test("jobs wire values match generated records and cursor input", () => {
  const timestamp = 1_800_000_000_000_000_001n;
  // Job records are entities keyed by attribute ids; unset optional fields
  // are omitted.
  const record: JobRecord = {
    type: "semantic:jobs:job",
    id: "job",
    "semantic:jobs:job:kind": "example",
    "semantic:jobs:job:status": "queued",
    "semantic:jobs:job:progress": { completed: 18_446_744_073_709_551_615n },
    "semantic:jobs:job:created_at": timestamp,
    "semantic:jobs:job:updated_at": timestamp,
    "semantic:jobs:job:snapshot_seq": 1,
  };
  const wire = encodeTagged(
    {
      ...record,
      "semantic:jobs:job:created_at": value.dateTimeNanos(timestamp),
      "semantic:jobs:job:updated_at": value.dateTimeNanos(timestamp),
    },
    "u64",
  );
  const decoded = decodeTagged(wire) as JobRecord;
  assert.deepEqual(
    {
      ...decoded,
      "semantic:jobs:job:progress": {
        ...decoded["semantic:jobs:job:progress"],
      },
    },
    record,
  );
  const page: JobListPage = {
    records: [record],
    next_cursor: { id: "job", created_at: timestamp },
  };
  const params: JobListParams = {
    cursor: {
      id: page.next_cursor!.id,
      created_at: value.dateTimeNanos(page.next_cursor!.created_at),
    },
  };
  assert.equal(
    stringifyJson(encodeTagged(params)),
    stringifyJson({
      object: {
        cursor: {
          object: {
            id: { string: "job" },
            created_at: { date_time: timestamp },
          },
        },
      },
    }),
  );
  const missing: JobsCommands["semantic.jobs.get"]["result"] = null;
  assert.equal(missing, null);
});
