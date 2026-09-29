import { command } from "../command.js";
import type {
  BatchOperation,
  BatchOutcome,
  BatchReturn,
  BatchReply,
  EntityRecord,
  FileAnalysisOutcome,
  QueryResult,
  ScopeInfo,
  SemanticObject,
  SemanticValue,
  ValidationViolation,
} from "../types.js";
export const commands = {
  scopeOpen: command<
    {
      uri: string;
      scope_id?: string;
      mode?: "open_existing" | "auto_create";
      visibility?: "principal" | "system";
      set_current?: boolean;
    },
    ScopeInfo & { current: boolean }
  >("semantic.scope.open"),
  scopeUse: command<
    string | { scope_id?: string } | null,
    { scope_id: string | null }
  >("semantic.scope.use"),
  scopeCurrent: command<SemanticObject | null, { scope_id: string | null }>(
    "semantic.scope.current",
  ),
  scopeList: command<SemanticObject | null, ScopeInfo[]>("semantic.scope.list"),
  catalog: command<
    { scope_id?: string },
    { format: "facet-json"; catalog: string }
  >("semantic.db.catalog"),
  query: command<
    {
      query: string;
      format?: "sql" | "prql";
      scope_id?: string;
      params?: Record<string, SemanticValue>;
    },
    QueryResult
  >("semantic.db.query"),
  get: command<
    { id: string; collection?: string; scope_id?: string },
    EntityRecord | null
  >("semantic.db.get"),
  insert: command<
    {
      id: string;
      object: SemanticObject;
      collection?: string;
      scope_id?: string;
    },
    undefined
  >("semantic.db.insert"),
  delete: command<
    { id: string; collection?: string; scope_id?: string },
    undefined
  >("semantic.db.delete"),
  batch: command<
    { operations: BatchOperation[]; scope_id?: string; returning?: "dataset" },
    BatchOutcome
  >("semantic.db.batch"),
  batchReturning: command<
    { operations: BatchOperation[]; scope_id?: string; returning: BatchReturn },
    BatchReply
  >("semantic.db.batch"),
  packageUpsert: command<
    { package: string; format?: "facet-json"; scope_id?: string },
    SemanticObject
  >("semantic.db.package.upsert"),
  validationPreflight: command<{ scope_id?: string }, ValidationViolation[]>(
    "semantic.db.validation.preflight",
  ),
  validationActivate: command<{ scope_id?: string }, undefined>(
    "semantic.db.validation.activate",
  ),
  fileAnalyze: command<{ id: string; scope_id?: string }, FileAnalysisOutcome>(
    "semantic.file.analyze",
  ),
} as const;
