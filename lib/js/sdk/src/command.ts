import type { CommandDefinition } from "./types.js";
export function command<P, O>(name: string): CommandDefinition<P, O> {
  return Object.freeze({ name }) as CommandDefinition<P, O>;
}
