import { ProtocolError, RpcError, TransportError } from "./errors.js";
import { parseJson, stringifyJson } from "./json.js";
import { RpcRequestEncoder, resolveRpcResponse } from "./rpc-wire.js";
import type { RpcResponseEnvelope } from "./rpc-wire.js";
import type { SemanticValue } from "./types.js";

import {
  abortable,
  abortError,
  fetchResponse,
  requestSignal,
  type HttpOptions,
  type InvokeOptions,
} from "./http-options.js";
export type { InvokeOptions } from "./http-options.js";

export interface RpcTransport {
  invoke(
    command: string,
    payload: SemanticValue,
    options?: InvokeOptions,
  ): Promise<SemanticValue | undefined>;
  close?(): void;
}
const requestEncoder = new RpcRequestEncoder();
const errorDescription = (error: unknown): string => {
  if (error instanceof Error) return `${error.name}: ${error.message}`;
  return String(error);
};

export interface HttpTransportOptions extends HttpOptions {}
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
    options: InvokeOptions = {},
  ): Promise<SemanticValue | undefined> {
    const cancellation = requestSignal(this.options.signal, options.signal);
    const signal = cancellation.signal;
    try {
      if (signal.aborted) throw abortError(signal);
      let response: Response;
      const headers = new Headers(this.options.headers);
      headers.set("content-type", "application/json");
      const init: RequestInit = {
        method: "POST",
        headers,
        body: stringifyJson(requestEncoder.request(command, payload)),
      };
      init.signal = signal;
      if (this.options.credentials) init.credentials = this.options.credentials;
      try {
        response = await fetchResponse(
          this.fetcher,
          this.endpoint,
          init,
          signal,
        );
      } catch (cause) {
        if (
          signal.aborted ||
          (cause instanceof Error && cause.name === "AbortError")
        )
          throw abortError(signal);
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
        return resolveRpcResponse(
          parseJson(await abortable(response.text(), signal)),
        );
      } catch (cause) {
        if (
          signal.aborted ||
          (cause instanceof Error && cause.name === "AbortError")
        )
          throw abortError(signal);
        if (cause instanceof RpcError) throw cause;
        throw new ProtocolError(
          `Invalid HTTP RPC response from ${this.endpoint}: ${errorDescription(cause)}`,
          { cause },
        );
      }
    } finally {
      cancellation.dispose();
    }
  }
}
