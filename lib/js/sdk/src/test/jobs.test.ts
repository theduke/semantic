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
  const record: JobRecord = {
    id: "job",
    "semantic:jobs:job:kind": "example",
    "semantic:jobs:job:status": "queued",
    "semantic:jobs:job:progress": {
      "semantic:jobs:job:progress:completed": 18_446_744_073_709_551_615n,
      "semantic:jobs:job:progress:total": null,
      "semantic:jobs:job:progress:unit": null,
      "semantic:jobs:job:progress:phase": null,
    },
    "semantic:jobs:job:error": null,
    "semantic:jobs:job:created_at": timestamp,
    "semantic:jobs:job:started_at": null,
    "semantic:jobs:job:updated_at": timestamp,
    "semantic:jobs:job:finished_at": null,
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
    "semantic:jobs:records": [record],
    "semantic:jobs:next_cursor": {
      id: "job",
      "semantic:jobs:job:created_at": timestamp,
    },
  };
  const cursor = page["semantic:jobs:next_cursor"]!;
  const params: JobListParams = {
    "semantic:jobs:cursor": {
      id: cursor.id,
      "semantic:jobs:job:created_at": value.dateTimeNanos(
        cursor["semantic:jobs:job:created_at"],
      ),
    },
  };
  assert.equal(
    stringifyJson(encodeTagged(params)),
    stringifyJson({
      object: {
        "semantic:jobs:cursor": {
          object: {
            id: { string: "job" },
            "semantic:jobs:job:created_at": { date_time: timestamp },
          },
        },
      },
    }),
  );
  const missing: JobsCommands["semantic.jobs.get"]["result"] = null;
  assert.equal(missing, null);
});
