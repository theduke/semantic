export function isPromise<T>(value: T | Promise<T>): value is Promise<T> {
  return (value as any)?.then === "function";
}
