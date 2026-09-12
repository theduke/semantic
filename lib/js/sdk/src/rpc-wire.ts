import { ProtocolError, RpcError } from "./errors.js";
import type { SemanticValue, TaggedValue } from "./types.js";
import { decodeTagged, encodeTagged } from "./values.js";

export interface RpcEnvelope {
  id: bigint;
  command: string;
  payload: TaggedValue;
}

export interface RpcResponseEnvelope {
  id: number | bigint;
  result:
    | { ok: TaggedValue }
    | { err: { code: string; message: string; data?: TaggedValue } };
}

const MAX_U64 = (1n << 64n) - 1n;

/** Allocates request envelopes with IDs accepted by Rust's u64 protocol type. */
export class RpcRequestEncoder {
  private nextId = 1n;

  request(command: string, payload: SemanticValue): RpcEnvelope {
    if (this.nextId > MAX_U64)
      throw new ProtocolError("RPC request ID space exhausted");
    return { id: this.nextId++, command, payload: encodeTagged(payload) };
  }
}

/** Decode an RPC response and preserve structured application errors. */
export function resolveRpcResponse(raw: unknown): SemanticValue | undefined {
  if (!raw || typeof raw !== "object" || !("result" in raw))
    throw new ProtocolError("invalid RPC response envelope");
  const response = raw as RpcResponseEnvelope;
  if (!response.result || typeof response.result !== "object")
    throw new ProtocolError("invalid RPC response result");
  if ("err" in response.result) {
    const error = response.result.err;
    throw new RpcError(
      error.code,
      error.message,
      error.data === undefined ? undefined : decodeTagged(error.data),
    );
  }
  if (!("ok" in response.result))
    throw new ProtocolError("RPC response has neither ok nor err result");
  return decodeTagged(response.result.ok);
}
