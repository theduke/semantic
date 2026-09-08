import type { SemanticValue } from "./types.js";
export interface RpcErrorBody {
  code: string;
  message: string;
  data?: SemanticValue;
}
export class SemanticError extends Error {
  constructor(message: string, options?: ErrorOptions) {
    super(message, options);
    this.name = new.target.name;
  }
}
export class TransportError extends SemanticError {
  constructor(
    message: string,
    public readonly status?: number,
    options?: ErrorOptions,
  ) {
    super(message, options);
  }
}
export class ProtocolError extends SemanticError {}
export class RpcError extends SemanticError {
  constructor(
    public readonly code: string,
    message: string,
    public readonly data?: SemanticValue,
  ) {
    super(message);
  }
}
