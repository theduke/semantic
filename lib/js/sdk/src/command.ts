import type { CommandDefinition, ValueDecoding } from "./types.js";
export function command<P, O>(
  name: string,
  options: { valueDecoding?: ValueDecoding } = {},
): CommandDefinition<P, O> {
  return Object.freeze({ name, ...options }) as CommandDefinition<P, O>;
}
