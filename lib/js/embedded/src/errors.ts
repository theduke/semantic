export type EmbeddedErrorCode =
  | "EMBEDDED_CLOSED"
  | "EMBEDDED_BUSY"
  | "EMBEDDED_OPEN_FAILED"
  | "EMBEDDED_LOAD_FAILED"
  | "EMBEDDED_VERSION_MISMATCH"
  | "EMBEDDED_INVALID_ARGUMENT"
  | "EMBEDDED_INVALID_OPTIONS"
  | "EMBEDDED_PROTOCOL_ERROR"
  | "EMBEDDED_FILE_ERROR";

export class EmbeddedError extends Error {
  constructor(
    public readonly code: EmbeddedErrorCode,
    message: string,
    options?: ErrorOptions,
  ) {
    super(message, options);
    this.name = "EmbeddedError";
  }
}

export function mapNativeError(
  cause: unknown,
  fallback: EmbeddedErrorCode,
): EmbeddedError {
  if (cause instanceof EmbeddedError) return cause;
  const native = cause as { code?: unknown; message?: unknown };
  const message =
    typeof native?.message === "string"
      ? native.message
      : "Embedded operation failed";
  const prefix = /^((?:EMBEDDED_)[A-Z_]+):/.exec(message)?.[1];
  const code =
    typeof native?.code === "string" && native.code.startsWith("EMBEDDED_")
      ? (native.code as EmbeddedErrorCode)
      : prefix
        ? (prefix as EmbeddedErrorCode)
        : fallback;
  return new EmbeddedError(code, message, { cause });
}
