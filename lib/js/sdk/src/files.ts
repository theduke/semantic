import { TransportError } from "./errors.js";
import { flatJson } from "./json.js";
import type { SemanticObject } from "./types.js";
export interface FileUploadRequest {
  content: BodyInit;
  scopeId?: string;
  id?: string;
  filename?: string;
  mimeType?: string;
  entity?: SemanticObject;
  onProgress?: (uploaded: number, total?: number) => void;
}
export interface FileUploadResponse {
  id: string;
  collection: string;
  object: SemanticObject;
}
type StreamingRequestInit = RequestInit & { duplex?: "half" };
export function deriveFileEndpoint(rpcEndpoint: string): string {
  if (rpcEndpoint.endsWith("/rpc")) return `${rpcEndpoint.slice(0, -4)}/file`;
  const url = new URL(rpcEndpoint, "http://semantic.local");
  const path = "/api/v1/file";
  return url.origin === "http://semantic.local" ? path : `${url.origin}${path}`;
}
export class FileClient {
  constructor(
    readonly endpoint: string,
    private readonly fetcher: typeof fetch = globalThis.fetch,
  ) {}
  async upload(request: FileUploadRequest): Promise<FileUploadResponse> {
    const url = new URL(
      this.endpoint,
      globalThis.location?.href ?? "http://semantic.local",
    );
    if (request.scopeId) url.searchParams.set("scope", request.scopeId);
    const headers = new Headers();
    headers.set(
      "x-semantic-file-entity",
      flatJson.stringify(request.entity ?? {}),
    );
    if (request.id) headers.set("x-semantic-file-id", request.id);
    if (request.filename) headers.set("x-semantic-filename", request.filename);
    if (request.mimeType) headers.set("content-type", request.mimeType);
    request.onProgress?.(0);
    const init: StreamingRequestInit = {
      method: "POST",
      headers,
      body: request.content,
    };
    // Fetch implementations that support streaming request bodies use the
    // half-duplex request mode. Browsers ignore this member where it is not
    // part of their RequestInit implementation.
    if (
      typeof ReadableStream === "function" &&
      request.content instanceof ReadableStream
    )
      init.duplex = "half";
    const response = await this.fetcher(
      url.origin === "http://semantic.local"
        ? `${url.pathname}${url.search}`
        : url,
      init,
    );
    if (!response.ok)
      throw new TransportError(
        `file upload failed with status ${response.status}`,
        response.status,
      );
    const result = flatJson.parse<FileUploadResponse>(await response.text());
    const total =
      request.content instanceof Blob ? request.content.size : undefined;
    request.onProgress?.(total ?? 0, total);
    return result;
  }
  url(id: string, scopeId?: string): string {
    const url = `${this.endpoint}/${encodeURIComponent(id)}`;
    return scopeId ? `${url}?scope=${encodeURIComponent(scopeId)}` : url;
  }
  async read(
    id: string,
    options: { scopeId?: string; offset?: number; size?: number } = {},
  ): Promise<Uint8Array> {
    const offset = options.offset ?? 0;
    if (!Number.isSafeInteger(offset) || offset < 0)
      throw new RangeError("file offset must be a non-negative safe integer");
    if (
      options.size !== undefined &&
      (!Number.isSafeInteger(options.size) || options.size < 0)
    )
      throw new RangeError("file size must be a non-negative safe integer");
    if (options.size === 0) return new Uint8Array();
    const headers = new Headers();
    const ranged = offset !== 0 || options.size !== undefined;
    const expectedEnd =
      options.size === undefined ? undefined : offset + options.size - 1;
    if (expectedEnd !== undefined && !Number.isSafeInteger(expectedEnd))
      throw new RangeError(
        "file byte range exceeds JavaScript's safe integer range",
      );
    if (ranged) headers.set("range", `bytes=${offset}-${expectedEnd ?? ""}`);
    const response = await this.fetcher(this.url(id, options.scopeId), {
      headers,
    });
    if (response.status === 416) return new Uint8Array();
    if (!response.ok)
      throw new TransportError(
        `file download failed with status ${response.status}`,
        response.status,
      );
    let declaredLength: number | undefined;
    if (ranged) {
      if (response.status !== 206)
        throw new TransportError(
          "file server ignored the requested byte range",
          response.status,
        );
      const match = /^bytes (\d+)-(\d+)\/(\d+|\*)$/.exec(
        response.headers.get("content-range") ?? "",
      );
      const returnedStart = Number(match?.[1]);
      const returnedEnd = Number(match?.[2]);
      const total = match?.[3] === "*" ? undefined : Number(match?.[3]);
      if (
        !match ||
        !Number.isSafeInteger(returnedStart) ||
        !Number.isSafeInteger(returnedEnd) ||
        (total !== undefined &&
          (!Number.isSafeInteger(total) || total <= returnedEnd)) ||
        returnedStart !== offset ||
        returnedEnd < returnedStart ||
        (expectedEnd !== undefined && returnedEnd > expectedEnd)
      )
        throw new TransportError(
          "file server returned an invalid Content-Range",
          response.status,
        );
      declaredLength = returnedEnd - returnedStart + 1;
    }
    const bytes = new Uint8Array(await response.arrayBuffer());
    if (declaredLength !== undefined && bytes.byteLength !== declaredLength)
      throw new TransportError(
        "file server returned an unexpected byte count",
        response.status,
      );
    return bytes;
  }
}
