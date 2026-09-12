import { createRequire } from "node:module";
import { EmbeddedError, mapNativeError } from "./errors.js";

export const PROTOCOL_VERSION = 1;
export interface NativeOptions {
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
export interface NativeEmbedded {
  invokeJson(request: string): Promise<string>;
  uploadFile(content: Uint8Array, optionsJson: string): Promise<string>;
  readFile(id: string, optionsJson: string): Promise<Uint8Array>;
  close(): Promise<void>;
}
interface NativeModule {
  open(options: NativeOptions): Promise<NativeEmbedded>;
  protocolVersion(): number;
  buildInfo?(): string;
}
let override: NativeModule | undefined;
/** @internal */
export function setNativeModuleForTest(module: NativeModule | undefined): void {
  override = module;
}
export function loadNative(): NativeModule {
  if (override) return override;
  try {
    return createRequire(import.meta.url)("../native/index.js") as NativeModule;
  } catch (cause) {
    throw new EmbeddedError(
      "EMBEDDED_LOAD_FAILED",
      "Unable to load the Semantic native addon for this platform",
      { cause },
    );
  }
}
export async function openNative(
  options: NativeOptions,
): Promise<NativeEmbedded> {
  const module = loadNative();
  const version = module.protocolVersion();
  if (version !== PROTOCOL_VERSION)
    throw new EmbeddedError(
      "EMBEDDED_VERSION_MISMATCH",
      `Native protocol ${version} does not match wrapper protocol ${PROTOCOL_VERSION}`,
    );
  try {
    return await module.open(options);
  } catch (cause) {
    throw mapNativeError(cause, "EMBEDDED_OPEN_FAILED");
  }
}
