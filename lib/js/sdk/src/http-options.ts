/** Shared connection settings. Request-specific protocol headers take precedence. */
export interface HttpOptions {
  fetch?: typeof fetch;
  headers?: HeadersInit;
  credentials?: RequestCredentials;
  signal?: AbortSignal;
  fileEndpoint?: string;
}

export interface InvokeOptions {
  signal?: AbortSignal;
}

export function abortError(signal?: AbortSignal): DOMException {
  const reason: unknown = signal?.reason;
  if (reason instanceof DOMException && reason.name === "AbortError")
    return reason;
  return new DOMException(
    reason instanceof Error ? reason.message : "The operation was aborted",
    "AbortError",
  );
}

/** Combine signals without retaining request listeners after completion. */
export function requestSignal(connection?: AbortSignal, request?: AbortSignal) {
  const signals = [
    ...new Set([connection, request].filter((s) => s !== undefined)),
  ];
  const controller = new AbortController();
  const abort = (event: Event) =>
    controller.abort((event.target as AbortSignal).reason);
  for (const signal of signals) {
    if (signal.aborted) {
      controller.abort(signal.reason);
      break;
    }
    signal.addEventListener("abort", abort, { once: true });
  }
  return {
    signal: controller.signal,
    dispose: () => {
      for (const signal of signals) signal.removeEventListener("abort", abort);
    },
  };
}

/** Also supports injected fetch implementations that do not reject on abort. */
export function abortable<T>(
  operation: Promise<T>,
  signal?: AbortSignal,
): Promise<T> {
  if (!signal) return operation;
  return new Promise((resolve, reject) => {
    const abort = () => {
      signal.removeEventListener("abort", abort);
      reject(abortError(signal));
    };
    if (signal.aborted) abort();
    else signal.addEventListener("abort", abort, { once: true });
    operation
      .then(resolve, reject)
      .finally(() => signal.removeEventListener("abort", abort));
  });
}

/** Dispose a late response even if a custom fetch ignores its AbortSignal. */
export function fetchResponse(
  fetcher: typeof fetch,
  input: RequestInfo | URL,
  init: RequestInit,
  signal: AbortSignal,
): Promise<Response> {
  const response = fetcher(input, init).then(async (response) => {
    if (signal.aborted) {
      await response.body?.cancel().catch(() => undefined);
      throw abortError(signal);
    }
    return response;
  });
  return abortable(response, signal);
}
