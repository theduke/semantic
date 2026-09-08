import type {
  BatchOperation,
  BatchOutcome,
  CommandDefinition,
  EntityRecord,
  Package,
  QueryResult,
  SemanticObject,
  SemanticValue,
} from "./types.js";
import type { RpcTransport } from "./transport.js";
import { packageJson, parseJson } from "./json.js";

const object = (v: SemanticValue | undefined): SemanticObject => {
  if (
    !v ||
    Array.isArray(v) ||
    v instanceof Map ||
    v instanceof Uint8Array ||
    v instanceof Date ||
    typeof v !== "object" ||
    "$variant" in v ||
    "$tagged" in v
  )
    throw new TypeError("RPC output is not an object");
  return v;
};
export interface RequestOptions {
  scopeId?: string;
}
export class SemanticClient {
  constructor(readonly transport: RpcTransport) {}
  invoke<P, O>(definition: CommandDefinition<P, O>, payload: P): Promise<O> {
    return this.transport.invoke(
      definition.name,
      payload as unknown as SemanticValue,
    ) as Promise<O>;
  }
  async query<T extends object = SemanticObject>(
    query: string,
    options: RequestOptions & { format?: "sql" | "prql" } = {},
  ): Promise<QueryResult<T>> {
    return object(
      await this.transport.invoke("semantic.db.query", {
        query,
        format: options.format ?? "sql",
        ...(options.scopeId ? { scope_id: options.scopeId } : {}),
      }),
    ) as unknown as QueryResult<T>;
  }
  async sql<T extends object = SemanticObject>(
    query: string,
    options?: RequestOptions,
  ): Promise<QueryResult<T>> {
    return this.query<T>(query, { ...options, format: "sql" });
  }
  async prql<T extends object = SemanticObject>(
    query: string,
    options?: RequestOptions,
  ): Promise<QueryResult<T>> {
    return this.query<T>(query, { ...options, format: "prql" });
  }
  async get<T extends object = SemanticObject>(
    id: string,
    options: RequestOptions & { collection?: string } = {},
  ): Promise<EntityRecord<T> | null> {
    const v = await this.transport.invoke("semantic.db.get", {
      id,
      ...(options.collection ? { collection: options.collection } : {}),
      ...(options.scopeId ? { scope_id: options.scopeId } : {}),
    });
    return v === null ? null : (object(v) as unknown as EntityRecord<T>);
  }
  async insert<T extends object>(
    id: string,
    entity: T,
    options: RequestOptions & { collection?: string } = {},
  ): Promise<void> {
    await this.transport.invoke("semantic.db.insert", {
      id,
      object: entity,
      ...(options.collection ? { collection: options.collection } : {}),
      ...(options.scopeId ? { scope_id: options.scopeId } : {}),
    } as unknown as SemanticObject);
  }
  async delete(
    id: string,
    options: RequestOptions & { collection?: string } = {},
  ): Promise<void> {
    await this.transport.invoke("semantic.db.delete", {
      id,
      ...(options.collection ? { collection: options.collection } : {}),
      ...(options.scopeId ? { scope_id: options.scopeId } : {}),
    });
  }
  async batch(
    operations: readonly BatchOperation[],
    options: RequestOptions = {},
  ): Promise<BatchOutcome> {
    return object(
      await this.transport.invoke("semantic.db.batch", {
        operations: [...operations],
        ...(options.scopeId ? { scope_id: options.scopeId } : {}),
      } as unknown as SemanticObject),
    ) as unknown as BatchOutcome;
  }
  async upsertPackage(
    pkg: Package,
    options: RequestOptions = {},
  ): Promise<unknown> {
    const out = object(
      await this.transport.invoke("semantic.db.package.upsert", {
        format: "facet-json",
        package: packageJson.stringify(pkg),
        ...(options.scopeId ? { scope_id: options.scopeId } : {}),
      }),
    );
    return parseJson(String(out.outcome));
  }
  close(): void {
    this.transport.close?.();
  }
}
