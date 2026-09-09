export const CONFIG_STORAGE_KEY = "semanticBookmarkConfig";

export interface ExtensionConfig {
  baseUrl: string;
}

export interface StorageArea {
  get(key: string): Promise<Record<string, unknown>>;
  set(items: Record<string, unknown>): Promise<void>;
}

export function normalizeBaseUrl(input: string): string {
  let url: URL;
  try {
    url = new URL(input.trim());
  } catch {
    throw new Error("Enter a valid Semantic application URL.");
  }
  if (url.protocol !== "http:" && url.protocol !== "https:") {
    throw new Error("The Semantic URL must use HTTP or HTTPS.");
  }
  if (!url.hostname)
    throw new Error("The Semantic URL must include a hostname.");
  if (url.username || url.password) {
    throw new Error("The Semantic URL cannot contain credentials.");
  }
  if (url.search || url.hash) {
    throw new Error("The Semantic URL cannot contain a query or fragment.");
  }
  url.pathname = url.pathname.replace(/\/+$/, "");
  return url.toString().replace(/\/$/, "");
}

export function rpcEndpoint(config: ExtensionConfig): string {
  return `${config.baseUrl}/api/v1/rpc`;
}

export function entityUrl(config: ExtensionConfig, id: string): string {
  return `${config.baseUrl}/entities/${encodeURIComponent(id)}`;
}

export function originPermission(baseUrl: string): string {
  return `${new URL(baseUrl).origin}/*`;
}

export function parseConfig(value: unknown): ExtensionConfig | null {
  if (!value || typeof value !== "object") return null;
  const baseUrl = (value as { baseUrl?: unknown }).baseUrl;
  if (typeof baseUrl !== "string") return null;
  try {
    return { baseUrl: normalizeBaseUrl(baseUrl) };
  } catch {
    return null;
  }
}

export async function loadConfig(
  storage: StorageArea,
): Promise<ExtensionConfig | null> {
  const stored = await storage.get(CONFIG_STORAGE_KEY);
  return parseConfig(stored[CONFIG_STORAGE_KEY]);
}
