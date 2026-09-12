import { RpcError, TransportError } from "./errors.js";
import { flatJson, parseJson } from "./json.js";
import { decodeTagged } from "./values.js";
import type { SemanticObject, TaggedValue } from "./types.js";
import type { RequestOptions } from "./client.js";
import {
  abortable,
  abortError,
  fetchResponse,
  requestSignal,
  type HttpOptions,
} from "./http-options.js";

export interface FileReadOptions extends RequestOptions {
  offset?: number;
  size?: number;
}

export interface FileStreamResult {
  stream: ReadableStream<Uint8Array>;
  status: number;
  contentType?: string;
  contentLength?: number;
  contentRange?: string;
  totalSize?: number;
}

export interface FileUploadRequest extends RequestOptions {
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

async function fileError(
  response: Response,
  signal: AbortSignal,
  operation: string,
): Promise<RpcError | TransportError> {
  try {
    const body = parseJson(await abortable(response.text(), signal)) as Record<
      string,
      unknown
    >;
    if (
      body &&
      typeof body.code === "string" &&
      typeof body.message === "string"
    )
      return new RpcError(
        body.code,
        body.message,
        body.data === undefined
          ? undefined
          : decodeTagged(body.data as TaggedValue),
      );
  } catch (cause) {
    if (
      signal.aborted ||
      (cause instanceof Error && cause.name === "AbortError")
    )
      throw abortError(signal);
  }
  return new TransportError(
    `file ${operation} failed with status ${response.status}`,
    response.status,
  );
}

export function deriveFileEndpoint(rpcEndpoint: string): string {
  if (rpcEndpoint.endsWith("/rpc")) return `${rpcEndpoint.slice(0, -4)}/file`;
  const url = new URL(rpcEndpoint, "http://semantic.local");
  const path = "/api/v1/file";
  return url.origin === "http://semantic.local" ? path : `${url.origin}${path}`;
}
export class FileClient {
  private readonly fetcher: typeof fetch;
  private readonly options: HttpOptions;

  constructor(
    readonly endpoint: string,
    options: typeof fetch | HttpOptions = {},
  ) {
    this.options = typeof options === "function" ? { fetch: options } : options;
    if (!this.options.fetch && !globalThis.fetch)
      throw new TransportError("fetch is not available");
    this.fetcher = this.options.fetch ?? globalThis.fetch.bind(globalThis);
  }
  async upload(request: FileUploadRequest): Promise<FileUploadResponse> {
    const cancellation = requestSignal(this.options.signal, request.signal);
    const signal = cancellation.signal;
    try {
      if (signal.aborted) throw abortError(signal);
      const url = new URL(
        this.endpoint,
        globalThis.location?.href ?? "http://semantic.local",
      );
      if (request.scopeId) url.searchParams.set("scope", request.scopeId);
      const headers = new Headers(this.options.headers);
      headers.set(
        "x-semantic-file-entity",
        flatJson.stringify(request.entity ?? {}),
      );
      if (request.id) headers.set("x-semantic-file-id", request.id);
      if (request.filename)
        headers.set("x-semantic-filename", request.filename);
      if (request.mimeType) headers.set("content-type", request.mimeType);
      request.onProgress?.(0);
      const init: StreamingRequestInit = {
        method: "POST",
        headers,
        body: request.content,
        signal,
      };
      if (this.options.credentials) init.credentials = this.options.credentials;
      // Fetch implementations that support streaming request bodies use the
      // half-duplex request mode. Browsers ignore this member where it is not
      // part of their RequestInit implementation.
      if (
        typeof ReadableStream === "function" &&
        request.content instanceof ReadableStream
      )
        init.duplex = "half";
      const response = await fetchResponse(
        this.fetcher,
        url.origin === "http://semantic.local"
          ? `${url.pathname}${url.search}`
          : url,
        init,
        signal,
      );
      if (!response.ok) throw await fileError(response, signal, "upload");
      const result = flatJson.parse<FileUploadResponse>(
        await abortable(response.text(), signal),
      );
      const total =
        request.content instanceof Blob ? request.content.size : undefined;
      request.onProgress?.(total ?? 0, total);
      return result;
    } catch (cause) {
      if (
        signal.aborted ||
        (cause instanceof Error && cause.name === "AbortError")
      )
        throw abortError(signal);
      if (cause instanceof TransportError || cause instanceof RpcError)
        throw cause;
      throw new TransportError("file upload failed", undefined, { cause });
    } finally {
      cancellation.dispose();
    }
  }
  url(id: string, scopeId?: string): string {
    const url = `${this.endpoint}/${encodeURIComponent(id)}`;
    return scopeId ? `${url}?scope=${encodeURIComponent(scopeId)}` : url;
  }

  /** Explicit native deletion; removing a domain file link does not delete a file. */
  async delete(id: string, options: RequestOptions = {}): Promise<void> {
    const cancellation = requestSignal(this.options.signal, options.signal);
    const signal = cancellation.signal;
    try {
      if (signal.aborted) throw abortError(signal);
      const init: RequestInit = {
        method: "DELETE",
        headers: new Headers(this.options.headers),
        signal,
      };
      if (this.options.credentials) init.credentials = this.options.credentials;
      const response = await fetchResponse(
        this.fetcher,
        this.url(id, options.scopeId),
        init,
        signal,
      );
      if (!response.ok) throw await fileError(response, signal, "delete");
      await response.body?.cancel();
      if (response.status !== 204)
        throw new TransportError(
          "file deletion did not return 204",
          response.status,
        );
    } catch (cause) {
      if (
        signal.aborted ||
        (cause instanceof Error && cause.name === "AbortError")
      )
        throw abortError(signal);
      if (cause instanceof TransportError || cause instanceof RpcError)
        throw cause;
      throw new TransportError("file deletion failed", undefined, { cause });
    } finally {
      cancellation.dispose();
    }
  }
  async readStream(
    id: string,
    options: FileReadOptions = {},
  ): Promise<FileStreamResult> {
    const offset = options.offset ?? 0;
    if (!Number.isSafeInteger(offset) || offset < 0)
      throw new RangeError("file offset must be a non-negative safe integer");
    if (
      options.size !== undefined &&
      (!Number.isSafeInteger(options.size) || options.size < 0)
    )
      throw new RangeError("file size must be a non-negative safe integer");
    if (options.size === 0) return emptyStream(200);
    const headers = new Headers(this.options.headers);
    // A base Range header must not silently turn an ordinary read into a partial one.
    headers.delete("range");
    const ranged = offset !== 0 || options.size !== undefined;
    const expectedEnd =
      options.size === undefined ? undefined : offset + (options.size - 1);
    if (expectedEnd !== undefined && !Number.isSafeInteger(expectedEnd))
      throw new RangeError(
        "file byte range exceeds JavaScript's safe integer range",
      );
    if (ranged) headers.set("range", `bytes=${offset}-${expectedEnd ?? ""}`);
    const cancellation = requestSignal(this.options.signal, options.signal);
    const signal = cancellation.signal;
    let response: Response | undefined;
    try {
      if (signal.aborted) throw abortError(signal);
      const init: RequestInit = { headers, signal };
      if (this.options.credentials) init.credentials = this.options.credentials;
      response = await fetchResponse(
        this.fetcher,
        this.url(id, options.scopeId),
        init,
        signal,
      );
      if (response.status === 416) {
        const result = emptyStream(416);
        const range = response.headers.get("content-range");
        if (range !== null) {
          const match = /^bytes \*\/(\d+)$/.exec(range);
          if (!match || !Number.isSafeInteger(Number(match[1])))
            throw new TransportError(
              "file server returned an invalid Content-Range",
              416,
            );
          result.contentRange = range;
          result.totalSize = Number(match[1]);
        }
        await response.body?.cancel();
        cancellation.dispose();
        return result;
      }
      if (!response.ok) throw await fileError(response, signal, "download");
      const result: Omit<FileStreamResult, "stream"> = {
        status: response.status,
      };
      const contentType = response.headers.get("content-type");
      if (contentType !== null) result.contentType = contentType;
      let declaredLength: number | undefined;
      const contentLength = response.headers.get("content-length");
      if (contentLength !== null) {
        if (
          !/^\d+$/.test(contentLength) ||
          !Number.isSafeInteger(Number(contentLength))
        )
          throw new TransportError(
            "file server returned an invalid Content-Length",
            response.status,
          );
        declaredLength = Number(contentLength);
        result.contentLength = declaredLength;
      }
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
        const rangeLength = returnedEnd - returnedStart + 1;
        if (
          !Number.isSafeInteger(rangeLength) ||
          (declaredLength !== undefined && declaredLength !== rangeLength)
        )
          throw new TransportError(
            "file server returned inconsistent range and length headers",
            response.status,
          );
        declaredLength = rangeLength;
        result.contentRange = response.headers.get("content-range")!;
        if (total !== undefined) result.totalSize = total;
      } else {
        if (response.status === 206 || response.headers.has("content-range"))
          throw new TransportError(
            "file server returned an unsolicited byte range",
            response.status,
          );
        if (declaredLength !== undefined) result.totalSize = declaredLength;
      }
      return {
        ...result,
        stream: validatedStream(
          response,
          declaredLength,
          signal,
          cancellation.dispose,
        ),
      };
    } catch (cause) {
      cancellation.dispose();
      await response?.body?.cancel().catch(() => undefined);
      if (
        signal.aborted ||
        (cause instanceof Error && cause.name === "AbortError")
      )
        throw abortError(signal);
      if (cause instanceof TransportError || cause instanceof RpcError)
        throw cause;
      throw new TransportError("file download failed", response?.status, {
        cause,
      });
    }
  }

  /** Buffer the whole response. Use readStream for bounded-memory consumption. */
  async read(id: string, options: FileReadOptions = {}): Promise<Uint8Array> {
    const { stream } = await this.readStream(id, options);
    const reader = stream.getReader();
    const chunks: Uint8Array[] = [];
    let size = 0;
    try {
      for (;;) {
        const { done, value } = await reader.read();
        if (done) break;
        chunks.push(value);
        size += value.byteLength;
      }
    } finally {
      reader.releaseLock();
    }
    const bytes = new Uint8Array(size);
    let offset = 0;
    for (const chunk of chunks) {
      bytes.set(chunk, offset);
      offset += chunk.byteLength;
    }
    return bytes;
  }
}

function emptyStream(status: number): FileStreamResult {
  return {
    status,
    contentLength: 0,
    stream: new ReadableStream<Uint8Array>({
      start: (controller) => controller.close(),
    }),
  };
}

function validatedStream(
  response: Response,
  expectedLength: number | undefined,
  signal: AbortSignal,
  dispose: () => void,
): ReadableStream<Uint8Array> {
  const reader = response.body?.getReader();
  let received = 0;
  let finished = false;
  let abort: () => void;
  const cleanup = () => {
    signal.removeEventListener("abort", abort);
    dispose();
    reader?.releaseLock();
  };
  const cancel = async (reason?: unknown) => {
    if (finished) return;
    finished = true;
    try {
      await reader?.cancel(reason);
    } finally {
      cleanup();
    }
  };
  return new ReadableStream<Uint8Array>(
    {
      start(controller) {
        abort = () => {
          const error = abortError(signal);
          controller.error(error);
          void cancel(error).catch(() => undefined);
        };
        if (signal.aborted) abort();
        else signal.addEventListener("abort", abort, { once: true });
      },
      async pull(controller) {
        try {
          const next = await reader?.read();
          if (finished) return;
          if (!next || next.done) {
            if (expectedLength !== undefined && received !== expectedLength)
              throw new TransportError(
                "file server returned an unexpected byte count",
                response.status,
              );
            finished = true;
            cleanup();
            controller.close();
            return;
          }
          received += next.value.byteLength;
          if (
            !Number.isSafeInteger(received) ||
            (expectedLength !== undefined && received > expectedLength)
          )
            throw new TransportError(
              "file server returned an unexpected byte count",
              response.status,
            );
          controller.enqueue(next.value);
        } catch (cause) {
          if (finished) return;
          const error =
            signal.aborted ||
            (cause instanceof Error && cause.name === "AbortError")
              ? abortError(signal)
              : cause instanceof TransportError
                ? cause
                : new TransportError(
                    "file response stream failed",
                    response.status,
                    { cause },
                  );
          controller.error(error);
          await cancel(error).catch(() => undefined);
        }
      },
      cancel,
    },
    { highWaterMark: 0 },
  );
}
