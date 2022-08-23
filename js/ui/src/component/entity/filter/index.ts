import { newSelect } from "semantic/dist/api";
import { Select } from "semantic/dist/core";
import {
  exprAndMany,
  exprAttr,
  exprContains,
  exprIsInEntityTypes,
  exprLiteral,
} from "semantic/dist/db";
import { SEMANTIC_TITLE } from "semantic/dist/schema";
import * as zod from "zod";

export const validateEntityFilterSql = zod.object({
  type: zod.literal("sql"),
  sql: zod.string(),
});

export interface EntityFilterSql {
  type: "sql";
  sql: string;
}

export const validateEntityFilterData = zod.object({
  type: zod.literal("data"),
  searchTerm: zod.optional(zod.string()),
  entityTypes: zod.optional(zod.array(zod.string())),
});

export interface EntityFilterData {
  type: "data";
  searchTerm?: string;
  entityTypes?: string[];
}

export function newFilterData(): EntityFilterData {
  return {
    type: "data",
    searchTerm: "",
  };
}

export function buildFilterDataSelect(filter: EntityFilterData): Select {
  let exprs = [];

  const term = filter.searchTerm?.trim();
  if (term) {
    const contains = exprContains(exprAttr(SEMANTIC_TITLE), exprLiteral(term));
    exprs.push(contains);
  }

  if (filter.entityTypes && filter.entityTypes.length > 0) {
    const types = filter.entityTypes;
    exprs.push(exprIsInEntityTypes(types));
  }

  return {
    ...newSelect(),
    filter: exprAndMany(exprs),
  };
}

export const validateEntityFilter = zod.discriminatedUnion("type", [
  validateEntityFilterData,
  validateEntityFilterSql,
]);

export type EntityFilter = EntityFilterData | EntityFilterSql;

export function filterIsSql(filter: EntityFilter): filter is EntityFilterSql {
  return filter.type === "sql";
}

export function filterIsData(filter: EntityFilter): filter is EntityFilterData {
  return filter.type === "data";
}
