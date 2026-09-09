import { ProtocolError, RpcError, TransportError } from "./errors.js";
import { parseJson, stringifyJson } from "./json.js";
import type { SemanticValue, TaggedValue } from "./types.js";
import { decodeTagged, encodeTagged } from "./values.js";

export interface RpcTransport {
  invoke(
    command: string,
    payload: SemanticValue,
  ): Promise<SemanticValue | undefined>;
  close?(): void;
}
interface RpcEnvelope {
  id: number | bigint;
  command: string;
  payload: TaggedValue;
}
interface RpcResponse {
  id: number | bigint;
  result:
    | { ok: TaggedValue }
    | { err: { code: string; message: string; data?: TaggedValue } };
}
let nextId = 1n;
const request = (command: string, payload: SemanticValue): RpcEnvelope => ({
  id: nextId++,
  command,
  payload: encodeTagged(payload),
});
const errorDescription = (error: unknown): string => {
  if (error instanceof Error) return `${error.name}: ${error.message}`;
  return String(error);
};
function resolve(raw: unknown): SemanticValue | undefined {
  const response = raw as RpcResponse;
  if (!response || typeof response !== "object" || !("result" in response))
    throw new ProtocolError("invalid RPC response envelope");
  if ("err" in response.result) {
    const e = response.result.err;
    throw new RpcError(
      e.code,
      e.message,
      e.data === undefined ? undefined : decodeTagged(e.data),
    );
  }
  if (!("ok" in response.result))
    throw new ProtocolError("RPC response has neither ok nor err result");
  return decodeTagged(response.result.ok);
}

export interface HttpTransportOptions {
  fetch?: typeof fetch;
  headers?: HeadersInit;
  signal?: AbortSignal;
}
export class HttpTransport implements RpcTransport {
  readonly endpoint: string;
  private readonly fetcher: typeof fetch;
  constructor(
    endpoint: string,
    private readonly options: HttpTransportOptions = {},
  ) {
    this.endpoint = endpoint;
    if (options.fetch) {
      this.fetcher = options.fetch;
    } else {
      if (!globalThis.fetch) throw new TransportError("fetch is not available");
      this.fetcher = globalThis.fetch.bind(globalThis);
    }
  }
  async invoke(
    command: string,
    payload: SemanticValue,
  ): Promise<SemanticValue | undefined> {
    let response: Response;
    const headers = new Headers(this.options.headers);
    headers.set("content-type", "application/json");
    const init: RequestInit = {
      method: "POST",
      headers,
      body: stringifyJson(request(command, payload)),
    };
    if (this.options.signal) init.signal = this.options.signal;
    try {
      response = await this.fetcher(this.endpoint, init);
    } catch (cause) {
      throw new TransportError(
        `HTTP RPC request to ${this.endpoint} failed: ${errorDescription(cause)}`,
        undefined,
        { cause },
      );
    }
    if (!response.ok)
      throw new TransportError(
        `HTTP RPC request to ${this.endpoint} failed: ${response.status}${response.statusText ? ` ${response.statusText}` : ""}`,
        response.status,
      );
    try {
      return resolve(parseJson(await response.text()));
    } catch (cause) {
      if (cause instanceof RpcError) throw cause;
      throw new ProtocolError(
        `Invalid HTTP RPC response from ${this.endpoint}: ${errorDescription(cause)}`,
        { cause },
      );
    }
  }
}

export interface WebSocketLike {
  readyState: number;
  send(data: string): void;
  close(code?: number, reason?: string): void;
  addEventListener(type: string, listener: (event: any) => void): void;
}
export type WebSocketFactory = (url: string) => WebSocketLike;
export class WebSocketTransport implements RpcTransport {
  private socket: WebSocketLike;
  private pending = new Map<
    string,
    { resolve(v: SemanticValue | undefined): void; reject(e: unknown): void }
  >();
  private state: "connecting" | "open" | "closed" = "connecting";
  private rejectReady: ((error: TransportError) => void) | undefined;
  readonly ready: Promise<void>;
  constructor(url: string, factory?: WebSocketFactory) {
    const socketFactory = factory ?? defaultWebSocketFactory();
    try {
      this.socket = socketFactory(url);
    } catch (cause) {
      if (cause instanceof TransportError) throw cause;
      throw new TransportError("WebSocket construction failed", undefined, {
        cause,
      });
    }
    this.ready = new Promise((resolveReady, rejectReady) => {
      let settled = false;
      const resolve = (): void => {
        if (settled || this.state === "closed") return;
        settled = true;
        this.rejectReady = undefined;
        this.state = "open";
        resolveReady();
      };
      const reject = (message: string): void => {
        if (settled) return;
        settled = true;
        this.rejectReady = undefined;
        this.state = "closed";
        rejectReady(new TransportError(message));
      };
      this.rejectReady = (error) => {
        if (settled) return;
        settled = true;
        this.rejectReady = undefined;
        rejectReady(error);
      };
      if (this.socket.readyState === 1) resolve();
      else if (this.socket.readyState >= 2)
        reject("WebSocket closed before opening");
      else {
        this.socket.addEventListener("open", resolve);
        this.socket.addEventListener("error", () =>
          reject("WebSocket connection failed"),
        );
        this.socket.addEventListener("close", () =>
          reject("WebSocket closed before opening"),
        );
      }
    });
    // The readiness promise remains rejectable for callers, while this attached
    // observer prevents a close/error before `ready` is consumed from becoming
    // an unhandled rejection.
    void this.ready.catch(() => undefined);
    this.socket.addEventListener("message", (event: MessageEvent) =>
      this.onMessage(typeof event.data === "string" ? event.data : ""),
    );
    this.socket.addEventListener("close", () => {
      this.state = "closed";
      this.failAll(new TransportError("WebSocket closed"));
    });
    this.socket.addEventListener("error", () => {
      this.state = "closed";
      this.failAll(new TransportError("WebSocket failed"));
    });
  }
  async invoke(
    command: string,
    payload: SemanticValue,
  ): Promise<SemanticValue | undefined> {
    if (this.state === "closed")
      throw new TransportError("WebSocket is closed");
    await this.ready;
    if (this.state !== "open" || this.socket.readyState !== 1)
      throw new TransportError("WebSocket is not open");
    const envelope = request(command, payload);
    const id = String(envelope.id);
    const promise = new Promise<SemanticValue | undefined>(
      (resolvePromise, reject) =>
        this.pending.set(id, { resolve: resolvePromise, reject }),
    );
    try {
      this.socket.send(stringifyJson(envelope));
    } catch (cause) {
      this.pending.delete(id);
      throw new TransportError("WebSocket send failed", undefined, { cause });
    }
    return promise;
  }
  close(): void {
    if (this.state === "closed") return;
    this.state = "closed";
    const error = new TransportError("WebSocket closed by client");
    this.rejectReady?.(error);
    this.failAll(error);
    this.socket.close();
  }
  private onMessage(text: string): void {
    try {
      const raw = parseJson(text) as RpcResponse;
      const pending = this.pending.get(String(raw.id));
      if (!pending) return;
      this.pending.delete(String(raw.id));
      try {
        pending.resolve(resolve(raw));
      } catch (e) {
        pending.reject(e);
      }
    } catch (cause) {
      this.failAll(
        new ProtocolError("invalid WebSocket RPC response", { cause }),
      );
    }
  }
  private failAll(error: Error): void {
    for (const p of this.pending.values()) p.reject(error);
    this.pending.clear();
  }
}

function defaultWebSocketFactory(): WebSocketFactory {
  const Socket = globalThis.WebSocket;
  if (typeof Socket !== "function")
    throw new TransportError(
      "WebSocket is not available in this runtime; provide a WebSocketFactory",
    );
  return (url) => new Socket(url);
}
