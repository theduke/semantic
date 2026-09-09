import type { QueryResult, SemanticObject } from "@semantic/sdk";
import type { WebBookmark } from "@semantic/sdk/generated/base";

import { entityUrl, rpcEndpoint, type ExtensionConfig } from "./config.js";

export const WEB_BOOKMARK_CLASS = "semantic:base:web_bookmark";
export const BOOKMARK_COLLECTION = "entities";

export interface BookmarkInput {
  url: string;
  title: string;
  description?: string;
}

export interface BookmarkLink {
  id: string;
  title?: string;
  href: string;
}

export interface BookmarkClient {
  sql<T extends object = SemanticObject>(
    query: string,
  ): Promise<QueryResult<T>>;
  insert<T extends object>(
    id: string,
    entity: T,
    options?: { collection?: string },
  ): Promise<void>;
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
  createId: () => string = () => crypto.randomUUID(),
): Promise<{ created: boolean; bookmark: BookmarkLink }> {
  const url = normalizeWebsiteUrl(input.url);
  const existing = await findBookmark(client, config, url);
  if (existing) return { created: false, bookmark: existing };

  const title = input.title.trim() || fallbackTitle(url);
  const description = input.description?.trim();
  const id = createId();
  const entity: WebBookmark = {
    type: WEB_BOOKMARK_CLASS,
    url,
    title,
    ...(description ? { description } : {}),
  };
  await client.insert(id, entity, { collection: BOOKMARK_COLLECTION });
  return {
    created: true,
    bookmark: { id, title, href: entityUrl(config, id) },
  };
}

const inFlightCaptures = new Map<
  string,
  Promise<{ created: boolean; bookmark: BookmarkLink }>
>();

export function captureBookmarkSerialized(
  createClient: BookmarkClientFactory,
  config: ExtensionConfig,
  input: BookmarkInput,
): Promise<{ created: boolean; bookmark: BookmarkLink }> {
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
