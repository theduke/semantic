export type { EntityType, ValueMap } from './db';

export * from './schema';

// Ignored because of some duplicate exports.
export type { Id, IdOrIdent } from './core';

import type { ValueMap } from './db';
import { BaseEntity, FACTOR_ID, FACTOR_IDENT, SEMANTIC_TITLE } from './schema';

export function genericTypedEntityTitle<E extends BaseEntity>(
  entity: E,
): string {
  const title = (entity as any as ValueMap)[SEMANTIC_TITLE];
  if (title) {
    return title.toString();
  } else if (FACTOR_ID in entity) {
    return 'Entity ' + entity[FACTOR_ID];
  } else {
    return '<no title>';
  }
}

export function genericEntityTitle(entity: ValueMap): string {
  const title = entity[SEMANTIC_TITLE];
  if (title) {
    return title.toString();
  } else if (FACTOR_ID in entity) {
    return 'Entity ' + entity[FACTOR_ID];
  } else {
    return '<no title>';
  }
}

export function entityLinkPath(entity: ValueMap): string {
  const target = entity[FACTOR_IDENT] ?? entity[FACTOR_ID];
  return '/entity/' + target;
}
