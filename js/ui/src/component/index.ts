import { v4 as uuidv4 } from "uuid";

export function isPromise<T>(value: T | Promise<T>): value is Promise<T> {
  return typeof (value as any)?.then === "function";
}

export function newUuid(): string {
  return uuidv4();
}
