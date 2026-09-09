import type { SemanticObject } from "@semantic/sdk";
import { escapeSqlString, type BookmarkClient } from "./bookmark.js";

export type MetadataKind = "directory" | "parent" | "labels";
export interface MetadataOption {
  id: string;
  title: string;
  detail: string;
  color?: string;
  exclusiveGroup?: string;
}

export function metadataSearchSql(
  kind: Exclude<MetadataKind, "labels">,
  query: string,
): string {
  const pattern = escapeSqlString(`%${query.trim()}%`);
  const filter =
    kind === "directory" ? "e.type IN ('semantic:base:directory') AND " : "";
  return `SELECT e.id AS id, e.title AS title FROM entities AS e WHERE ${filter}(e.title ILIKE '${pattern}' OR e.id ILIKE '${pattern}') ORDER BY e.title ASC, e.id ASC LIMIT 30`;
}

export async function searchMetadata(
  client: BookmarkClient,
  kind: MetadataKind,
  query: string,
): Promise<MetadataOption[]> {
  if (kind === "labels") {
    const value = await client.transport.invoke(
      "semantic.base.labels.list",
      {},
    );
    if (!Array.isArray(value)) throw new Error("Unexpected label response.");
    return labelOptions(value as SemanticObject[]).filter((option) =>
      `${option.title} ${option.detail}`
        .toLocaleLowerCase()
        .includes(query.trim().toLocaleLowerCase()),
    );
  }
  const result = await client.sql<{ id?: unknown; title?: unknown }>(
    metadataSearchSql(kind, query),
  );
  if (result.kind !== "select") throw new Error("Unexpected search response.");
  return result.rows.flatMap((row) =>
    typeof row.id === "string" && row.id
      ? [
          {
            id: row.id,
            title:
              typeof row.title === "string" && row.title ? row.title : row.id,
            detail: row.id,
          },
        ]
      : [],
  );
}

export function labelOptions(rows: SemanticObject[]): MetadataOption[] {
  const catalog = new Map(
    rows
      .filter((row) => row && typeof row.id === "string")
      .map((row) => [String(row.id), row]),
  );
  return [...catalog.values()]
    .filter((row) => row.type === "semantic:base:label")
    .map((row) => {
      const parents: string[] = [];
      const seen = new Set([row.id]);
      let parent = catalog.get(String(row["semantic:parent"]));
      const group =
        parent?.type === "semantic:base:label_group" &&
        parent["semantic:base:label:selection_mode"] === "exclusive"
          ? String(parent.id)
          : undefined;
      while (parent && !seen.has(parent.id)) {
        seen.add(parent.id);
        parents.unshift(
          String(parent["semantic:base:label:name"] ?? parent.id),
        );
        parent = catalog.get(String(parent["semantic:parent"]));
      }
      const color = row["semantic:base:label:color"];
      return {
        id: String(row.id),
        title: String(row["semantic:base:label:name"] ?? row.id),
        detail: parents.join(" / ") + (group ? " · Choose one" : ""),
        ...(typeof color === "string" && /^#[0-9a-f]{6}$/i.test(color)
          ? { color }
          : {}),
        ...(group ? { exclusiveGroup: group } : {}),
      };
    })
    .sort((a, b) =>
      `${a.detail}/${a.title}`.localeCompare(`${b.detail}/${b.title}`),
    );
}

export function selectLabel(
  selected: MetadataOption[],
  option: MetadataOption,
): MetadataOption[] {
  return [
    ...selected.filter(
      (item) =>
        item.id !== option.id &&
        (!option.exclusiveGroup ||
          item.exclusiveGroup !== option.exclusiveGroup),
    ),
    option,
  ];
}
