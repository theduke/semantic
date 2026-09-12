import { resolve } from "node:path";
import { SemanticClient } from "@semantic/sdk";
import { EmbeddedError } from "./errors.js";
import { EmbeddedFileClient } from "./files.js";
import { openNative, type NativeOptions } from "./native.js";
import { EmbeddedTransport } from "./transport.js";

export { EmbeddedError, type EmbeddedErrorCode } from "./errors.js";
export {
  EmbeddedFileClient,
  type EmbeddedFileContent,
  type EmbeddedFileUploadRequest,
  type EmbeddedFileUploadResponse,
} from "./files.js";
export { EmbeddedTransport } from "./transport.js";
export interface EmbeddedOptions {
  dataDir: string;
  dbUri?: string;
  blobUri?: string;
  blobPassword?: string;
  mode?: "openExisting" | "autoCreate";
  tempDir?: string;
  autoAnalyzeMedia?: boolean;
  maxConcurrentRequests?: number;
  maxBufferedFileBytes?: number;
}
export interface EmbeddedSemantic {
  readonly client: SemanticClient;
  readonly transport: EmbeddedTransport;
  readonly files: EmbeddedFileClient;
  close(): Promise<void>;
}
const DEFAULT_MAX_BUFFERED_FILE_BYTES = 64 * 1024 * 1024;

export async function openEmbedded(
  options: EmbeddedOptions,
): Promise<EmbeddedSemantic> {
  validateOptions(options);
  const nativeOptions: NativeOptions = {
    ...options,
    dataDir: resolve(options.dataDir),
    ...(options.tempDir === undefined
      ? {}
      : { tempDir: resolve(options.tempDir) }),
  };
  const native = await openNative(nativeOptions);
  const transport = new EmbeddedTransport(native);
  return {
    client: new SemanticClient(transport),
    transport,
    files: new EmbeddedFileClient(
      native,
      options.maxBufferedFileBytes ?? DEFAULT_MAX_BUFFERED_FILE_BYTES,
    ),
    close: () => transport.closeAsync(),
  };
}

function validateOptions(options: EmbeddedOptions): void {
  if (
    !options ||
    typeof options.dataDir !== "string" ||
    options.dataDir.length === 0
  )
    throw new EmbeddedError("EMBEDDED_INVALID_ARGUMENT", "dataDir is required");
  for (const [name, value] of [
    ["maxConcurrentRequests", options.maxConcurrentRequests],
    ["maxBufferedFileBytes", options.maxBufferedFileBytes],
  ] as const) {
    if (value !== undefined && (!Number.isSafeInteger(value) || value <= 0))
      throw new EmbeddedError(
        "EMBEDDED_INVALID_ARGUMENT",
        `${name} must be a positive safe integer`,
      );
  }
  if (
    options.blobPassword !== undefined &&
    !options.blobUri?.startsWith("logfs://")
  )
    throw new EmbeddedError(
      "EMBEDDED_INVALID_ARGUMENT",
      "blobPassword is only valid with a logfs blob URI",
    );
}
