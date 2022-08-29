import { newSelect } from "semantic/dist/api";
import { Expr, Select } from "semantic/dist/core";
import {
  exprAnd,
  exprAttr,
  exprIsEntityType,
  exprRegexIMatch,
} from "semantic/dist/db";
import { SEMANTIC_TAG_NAME, TY_SEMANTIC_TAG } from "semantic/dist/schema";

// Builds a Select for tags, sorted by name.
export function buildTagSelect(): Select {
  return {
    ...newSelect(),
    // FIXME: no bigint (needs type change)
    limit: 1000 as any,
    filter: exprIsEntityType(TY_SEMANTIC_TAG),
    sort: [{ on: exprAttr(SEMANTIC_TAG_NAME), order: "Asc" }],
  };
}

export function exprSearchTagByName(term: string): Expr {
  return exprAnd(
    exprIsEntityType(TY_SEMANTIC_TAG),
    exprRegexIMatch(exprAttr(SEMANTIC_TAG_NAME), term)
  );
}
