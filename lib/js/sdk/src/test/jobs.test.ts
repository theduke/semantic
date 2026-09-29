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
    kind: "example",
    status: "queued",
    progress: {
      completed: 18_446_744_073_709_551_615n,
      total: null,
      unit: null,
      phase: null,
    },
    error: null,
    created_at: timestamp,
    started_at: null,
    updated_at: timestamp,
    finished_at: null,
    snapshot_seq: 1,
  };
  const wire = encodeTagged(
    {
      ...record,
      created_at: value.dateTimeNanos(timestamp),
      updated_at: value.dateTimeNanos(timestamp),
    },
    "u64",
  );
  const decoded = decodeTagged(wire) as JobRecord;
  assert.deepEqual({ ...decoded, progress: { ...decoded.progress } }, record);
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
