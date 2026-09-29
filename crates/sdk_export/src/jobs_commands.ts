
/** Jobs commands accept decoded SemanticValue payloads and return decoded values. */
export type JobScopeParams = { scope_id?: string | null };
/** Encode cursor timestamps with value.dateTimeNanos(next_cursor.created_at). */
export type JobListCursorInput = Omit<JobListCursor, "created_at"> & {
  created_at: Date | import("../types.js").EncodedTaggedValue;
};
export type JobListParams = JobScopeParams & Partial<Omit<JobListQuery, "cursor">> & {
  cursor?: JobListCursorInput | null;
};
export type JobIdParams = JobScopeParams & { id: string };
export type JobsCommands = {
  "semantic.jobs.list": { params: JobListParams; result: JobListPage };
  "semantic.jobs.get": { params: JobIdParams; result: JobRecord | null };
  "semantic.jobs.cancel": { params: JobIdParams; result: JobRecord };
  "semantic.jobs.clear_completed": { params: JobScopeParams; result: ClearCompletedResult };
  "semantic.jobs.kinds": { params: JobScopeParams; result: Array<JobKindDescriptor> };
};
