import type { SemanticObject, TaggedValue } from "@semantic/sdk";
import {
  decodeTagged,
  encodeTagged,
  parseJson,
  stringifyJson,
} from "@semantic/sdk";
import { EmbeddedError, mapNativeError } from "./errors.js";
import type { NativeEmbedded } from "./native.js";

export type EmbeddedFileContent = Uint8Array | ArrayBuffer | Blob | string;
export interface EmbeddedFileUploadRequest {
  content: EmbeddedFileContent;
  scopeId?: string;
  id?: string;
  filename?: string;
  mimeType?: string;
  entity?: SemanticObject;
  onProgress?: (uploaded: number, total?: number) => void;
}
export interface EmbeddedFileUploadResponse {
  id: string;
  collection: string;
  object: SemanticObject;
}
interface NativeUploadResponse {
  id: string;
  collection: string;
  object: TaggedValue;
}

export class EmbeddedFileClient {
  constructor(
    private readonly native: NativeEmbedded,
    private readonly maxBufferedBytes: number,
  ) {}
  async upload(
    request: EmbeddedFileUploadRequest,
  ): Promise<EmbeddedFileUploadResponse> {
    const content = await this.toBytes(request.content);
    this.checkSize(content.byteLength);
    const options = {
      scopeId: request.scopeId,
      id: request.id,
      filename: request.filename,
      mimeType: request.mimeType,
      entity:
        request.entity === undefined ? undefined : encodeTagged(request.entity),
    };
    try {
      const raw = parseJson(
        await this.native.uploadFile(content, stringifyJson(options)),
      ) as NativeUploadResponse;
      if (
        !raw ||
        typeof raw.id !== "string" ||
        typeof raw.collection !== "string"
      )
        throw new EmbeddedError(
          "EMBEDDED_INVALID_ARGUMENT",
          "Invalid native file response",
        );
      const object = decodeTagged(raw.object);
      if (!object || typeof object !== "object" || Array.isArray(object))
        throw new EmbeddedError(
          "EMBEDDED_INVALID_ARGUMENT",
          "Invalid native file entity",
        );
      request.onProgress?.(content.byteLength, content.byteLength);
      return {
        id: raw.id,
        collection: raw.collection,
        object: object as SemanticObject,
      };
    } catch (cause) {
      throw mapNativeError(cause, "EMBEDDED_CLOSED");
    }
  }
  async read(
    id: string,
    options: { scopeId?: string; offset?: number; size?: number } = {},
  ): Promise<Uint8Array> {
    if (!id)
      throw new EmbeddedError(
        "EMBEDDED_INVALID_ARGUMENT",
        "File ID is required",
      );
    this.checkInteger("offset", options.offset);
    this.checkInteger("size", options.size);
    if (options.size !== undefined) this.checkSize(options.size);
    const offset = options.offset ?? 0;
    const end = options.size === undefined ? undefined : offset + options.size;
    if (end !== undefined && !Number.isSafeInteger(end))
      throw new EmbeddedError(
        "EMBEDDED_INVALID_ARGUMENT",
        "File range exceeds the safe integer limit",
      );
    try {
      const bytes = new Uint8Array(
        await this.native.readFile(
          id,
          stringifyJson({
            scopeId: options.scopeId,
            offset: options.offset,
            end,
          }),
        ),
      );
      this.checkSize(bytes.byteLength);
      return bytes;
    } catch (cause) {
      throw mapNativeError(cause, "EMBEDDED_CLOSED");
    }
  }
  private async toBytes(content: EmbeddedFileContent): Promise<Uint8Array> {
    if (typeof content === "string") return new TextEncoder().encode(content);
    if (content instanceof Uint8Array) return new Uint8Array(content);
    if (content instanceof ArrayBuffer) return new Uint8Array(content.slice(0));
    if (typeof Blob !== "undefined" && content instanceof Blob) {
      this.checkSize(content.size);
      return new Uint8Array(await content.arrayBuffer());
    }
    throw new EmbeddedError(
      "EMBEDDED_INVALID_ARGUMENT",
      "File content must be a Uint8Array, ArrayBuffer, Blob, or string",
    );
  }
  private checkInteger(name: string, value: number | undefined): void {
    if (value !== undefined && (!Number.isSafeInteger(value) || value < 0))
      throw new EmbeddedError(
        "EMBEDDED_INVALID_ARGUMENT",
        `${name} must be a nonnegative safe integer`,
      );
  }
  private checkSize(size: number): void {
    if (size > this.maxBufferedBytes)
      throw new EmbeddedError(
        "EMBEDDED_INVALID_ARGUMENT",
        `Buffered file size exceeds the ${this.maxBufferedBytes} byte limit`,
      );
  }
}
