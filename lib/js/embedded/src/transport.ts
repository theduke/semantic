import {
  ProtocolError,
  RpcError,
  parseJson,
  stringifyJson,
  type RpcTransport,
  type SemanticValue,
} from "@semantic/sdk";
import { RpcRequestEncoder, resolveRpcResponse } from "@semantic/sdk/rpc-wire";
import { EmbeddedError, mapNativeError } from "./errors.js";
import type { NativeEmbedded } from "./native.js";

export class EmbeddedTransport implements RpcTransport {
  private readonly encoder = new RpcRequestEncoder();
  private state: "open" | "closing" | "closed" = "open";
  private closePromise: Promise<void> | undefined;
  private resolveClosed!: () => void;
  private rejectClosed!: (error: unknown) => void;
  readonly closed: Promise<void>;
  constructor(private readonly native: NativeEmbedded) {
    this.closed = new Promise<void>((resolve, reject) => {
      this.resolveClosed = resolve;
      this.rejectClosed = reject;
    });
    void this.closed.catch(() => undefined);
  }
  async invoke(
    command: string,
    payload: SemanticValue,
  ): Promise<SemanticValue | undefined> {
    if (this.state !== "open")
      throw new EmbeddedError(
        "EMBEDDED_CLOSED",
        "Embedded Semantic is closing or closed",
      );
    let text: string;
    try {
      text = await this.native.invokeJson(
        stringifyJson(this.encoder.request(command, payload)),
      );
    } catch (cause) {
      if (cause instanceof EmbeddedError) throw cause;
      throw mapNativeError(cause, "EMBEDDED_CLOSED");
    }
    try {
      return resolveRpcResponse(parseJson(text));
    } catch (cause) {
      if (cause instanceof RpcError || cause instanceof ProtocolError)
        throw cause;
      throw new ProtocolError("Invalid embedded RPC response", { cause });
    }
  }
  close(): void {
    void this.closeAsync();
  }
  closeAsync(): Promise<void> {
    if (this.closePromise) return this.closePromise;
    this.state = "closing";
    this.closePromise = this.native.close().then(
      () => {
        this.state = "closed";
        this.resolveClosed();
      },
      (cause) => {
        this.state = "closed";
        const error = mapNativeError(cause, "EMBEDDED_CLOSED");
        this.rejectClosed(error);
        throw error;
      },
    );
    void this.closePromise.catch(() => undefined);
    return this.closePromise;
  }
}
