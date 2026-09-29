/** Jobs commands accept decoded SemanticValue payloads and return decoded values, keyed by attribute ids. */
export type JobScopeParams = { "semantic:scope:id"?: string | null };
/** Encode cursor timestamps with value.dateTimeNanos(next_cursor["semantic:jobs:job:created_at"]). */
export type JobListCursorInput = Omit<JobListCursor, "semantic:jobs:job:created_at"> & {
  "semantic:jobs:job:created_at": Date | import("../types.js").EncodedTaggedValue;
};
export type JobListParams = JobScopeParams & Partial<Omit<JobListQuery, "semantic:jobs:cursor">> & {
  "semantic:jobs:cursor"?: JobListCursorInput | null;
};
export type JobIdParams = JobScopeParams & { id: string };
export type JobsCommands = {
  "semantic.jobs.list": { params: JobListParams; result: JobListPage };
  "semantic.jobs.get": { params: JobIdParams; result: JobRecord | null };
  "semantic.jobs.cancel": { params: JobIdParams; result: JobRecord };
  "semantic.jobs.clear_completed": { params: JobScopeParams; result: ClearCompletedResult };
  "semantic.jobs.kinds": { params: JobScopeParams; result: Array<JobKindDescriptor> };
};
