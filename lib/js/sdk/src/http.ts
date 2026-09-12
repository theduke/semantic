import { SemanticClient } from "./client.js";
import { deriveFileEndpoint, FileClient } from "./files.js";
import type { HttpOptions } from "./http-options.js";
import { HttpTransport } from "./transport.js";

export type { HttpOptions } from "./http-options.js";

export function createHttpClient(
  rpcEndpoint: string,
  options: HttpOptions = {},
): { client: SemanticClient; files: FileClient } {
  return {
    client: new SemanticClient(new HttpTransport(rpcEndpoint, options)),
    files: new FileClient(
      options.fileEndpoint ?? deriveFileEndpoint(rpcEndpoint),
      options,
    ),
  };
}
