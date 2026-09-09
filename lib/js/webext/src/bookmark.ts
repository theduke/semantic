import type {
  BatchOperation,
  BatchOutcome,
  RpcTransport,
  QueryResult,
  SemanticObject,
} from "@semantic/sdk";
import type { WebBookmark } from "@semantic/sdk/generated/base";

import { entityUrl, rpcEndpoint, type ExtensionConfig } from "./config.js";

export const WEB_BOOKMARK_CLASS = "semantic:base:web_bookmark";
export const BOOKMARK_COLLECTION = "entities";

export interface BookmarkInput {
  url: string;
  title: string;
  description?: string;
  directoryId?: string;
  parentId?: string;
  labelIds?: string[];
}

export interface BookmarkLink {
  id: string;
  title?: string;
  href: string;
}

export interface BookmarkClient {
  transport: RpcTransport;
  batch(operations: readonly BatchOperation[]): Promise<BatchOutcome>;
  sql<T extends object = SemanticObject>(
    query: string,
  ): Promise<QueryResult<T>>;
  insert<T extends object>(
    id: string,
    entity: T,
    options?: { collection?: string },
  ): Promise<void>;
}

export interface BookmarkCaptureResult {
  created: boolean;
  bookmark: BookmarkLink;
  warning?: string;
}

export type BookmarkClientFactory = (endpoint: string) => BookmarkClient;

interface BookmarkRow {
  id?: unknown;
  title?: unknown;
}

export function normalizeWebsiteUrl(input: string): string {
  let url: URL;
  try {
    url = new URL(input);
  } catch {
    throw new Error("The active tab does not have a valid website URL.");
  }
  if (url.protocol !== "http:" && url.protocol !== "https:") {
    throw new Error("Only HTTP and HTTPS websites can be bookmarked.");
  }
  if (url.username || url.password) {
    throw new Error(
      "Website URLs containing credentials cannot be bookmarked.",
    );
  }
  return url.href;
}

export function fallbackTitle(url: string): string {
  return new URL(normalizeWebsiteUrl(url)).hostname;
}

export function createWebBookmarkId(
  uuid: string = crypto.randomUUID(),
): string {
  return `webbookmark-${uuid.replaceAll("-", "").slice(0, 12)}`;
}

export function escapeSqlString(value: string): string {
  return value.replaceAll("'", "''");
}

export function bookmarkLookupSql(url: string): string {
  const escapedUrl = escapeSqlString(normalizeWebsiteUrl(url));
  return `SELECT e.id AS id, e.title AS title FROM entities AS e WHERE e.type IN ('${WEB_BOOKMARK_CLASS}') AND (e.url = '${escapedUrl}' OR e."semantic:url" = '${escapedUrl}') ORDER BY e.id ASC LIMIT 1`;
}

export async function findBookmark(
  client: BookmarkClient,
  config: ExtensionConfig,
  url: string,
): Promise<BookmarkLink | null> {
  const result = await client.sql<BookmarkRow>(bookmarkLookupSql(url));
  if (result.kind !== "select" || !Array.isArray(result.rows)) {
    throw new Error("Semantic returned an unexpected bookmark query result.");
  }
  const row = result.rows[0];
  if (!row) return null;
  if (typeof row.id !== "string" || row.id.length === 0) {
    throw new Error("Semantic returned a bookmark without a valid ID.");
  }
  return {
    id: row.id,
    ...(typeof row.title === "string" && row.title.length > 0
      ? { title: row.title }
      : {}),
    href: entityUrl(config, row.id),
  };
}

export async function captureBookmark(
  client: BookmarkClient,
  config: ExtensionConfig,
  input: BookmarkInput,
  createId: () => string = createWebBookmarkId,
): Promise<BookmarkCaptureResult> {
  const url = normalizeWebsiteUrl(input.url);
  const existing = await findBookmark(client, config, url);
  if (existing) return { created: false, bookmark: existing };

  const title = input.title.trim() || fallbackTitle(url);
  const description = input.description?.trim();
  const id = createId();
  const entity: WebBookmark = {
    id,
    type: WEB_BOOKMARK_CLASS,
    url,
    title,
    ...(description ? { description } : {}),
    ...(input.parentId ? { ["semantic:parent"]: input.parentId } : {}),
  };
  if (input.directoryId) {
    const directory = escapeSqlString(input.directoryId);
    const target = await client.sql(
      `SELECT id FROM entities WHERE id = '${directory}' AND type IN ('semantic:base:directory') LIMIT 1`,
    );
    if (target.kind !== "select" || !target.rows.length)
      throw new Error(
        "The selected folder no longer exists. Choose another folder.",
      );
    const orderResult = await client.sql<{ next_order?: number | bigint }>(
      `SELECT MAX("semantic:base:directory_node:order") AS next_order FROM entities WHERE "semantic:base:directory_node:from" = '${directory}'`,
    );
    const lastOrder =
      orderResult.kind === "select"
        ? orderResult.rows[0]?.next_order
        : undefined;
    const order =
      typeof lastOrder === "bigint"
        ? lastOrder + 1n
        : typeof lastOrder === "number"
          ? lastOrder + 1
          : 0;
    const hex = (value: string) =>
      [...new TextEncoder().encode(value)]
        .map((byte) => byte.toString(16).padStart(2, "0"))
        .join("");
    const nodeId = `semantic:directory_node:${hex(input.directoryId)}:${hex(id)}`;
    await client.batch([
      { kind: "upsert", collection: BOOKMARK_COLLECTION, id, object: entity },
      {
        kind: "upsert",
        collection: BOOKMARK_COLLECTION,
        id: nodeId,
        object: {
          id: nodeId,
          type: "semantic:base:directory_node",
          "semantic:relation:relation": "semantic:base:directory_node",
          "semantic:base:directory_node:from": input.directoryId,
          "semantic:relation:to": id,
          "semantic:base:directory_node:order": order,
        },
      },
    ]);
  } else {
    await client.insert(id, entity, { collection: BOOKMARK_COLLECTION });
  }
  let warning: string | undefined;
  if (input.labelIds?.length) {
    try {
      await client.transport.invoke("semantic.base.labels.replace", {
        id,
        collection: BOOKMARK_COLLECTION,
        label_ids: [...new Set(input.labelIds)],
      });
    } catch {
      warning =
        "Bookmark saved, but labels could not be applied. Open the bookmark to add them.";
    }
  }
  return {
    created: true,
    bookmark: { id, title, href: entityUrl(config, id) },
    ...(warning ? { warning } : {}),
  };
}

const inFlightCaptures = new Map<string, Promise<BookmarkCaptureResult>>();

export function captureBookmarkSerialized(
  createClient: BookmarkClientFactory,
  config: ExtensionConfig,
  input: BookmarkInput,
): Promise<BookmarkCaptureResult> {
  const url = normalizeWebsiteUrl(input.url);
  const key = `${config.baseUrl}\0${url}`;
  const current = inFlightCaptures.get(key);
  if (current) return current;

  const pending = captureBookmark(createClient(rpcEndpoint(config)), config, {
    ...input,
    url,
  }).finally(() => inFlightCaptures.delete(key));
  inFlightCaptures.set(key, pending);
  return pending;
}
