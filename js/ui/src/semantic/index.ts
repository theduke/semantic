import { ValueMap } from "./registry";
import { BaseEntity, FACTOR_ID, FACTOR_IDENT, SEMANTIC_TITLE } from "semantic/dist/schema";

export const UUID_ZERO = "00000000-0000-0000-0000-000000000000";

export type EntityType = string;
export type AttributeName = string;

export function genericTypedEntityTitle<E extends BaseEntity>(
  entity: E
): string {
  const title = (entity as any as ValueMap)[SEMANTIC_TITLE];
  if (title) {
    return title.toString();
  } else if (FACTOR_ID in entity) {
    return "Entity " + entity[FACTOR_ID];
  } else {
    return "<no title>";
  }
}

export function genericEntityTitle(entity: ValueMap): string {
  const title = entity[SEMANTIC_TITLE];
  if (title) {
    return title.toString();
  } else if (FACTOR_ID in entity) {
    return "Entity " + entity[FACTOR_ID];
  } else {
    return "<no title>";
  }
}

export function entityLinkPath(entity: ValueMap): string {
  const target = entity[FACTOR_IDENT] ?? entity[FACTOR_ID];
  return "/entity/" + target;
}
