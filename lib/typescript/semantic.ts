// FIXME: generate interfaces from Rust types

export const ID_ZERO = "00000000-0000-0000-0000-000000000000";

export interface JoinItem<T = DataMap> {
  name: string;
  items: Item<T>[];
}

export type DataMap = Record<string, any>;

export interface Item<T = DataMap> {
  data: T;
  joins?: JoinItem[] | null;
}

export interface RelatedUrl {
  label: string;
  url: string;
}

export interface FetchUrlJob {
  url: string;
}

export interface FetchUrlOutput {
  items: Item[];
  load_more_url?: RelatedUrl | null;
  related_urls?: RelatedUrl[] | null;
  related_items?: Item[] | null;
}

export interface DbSchema {
}

export type UrlSupportMatcher = "All" | { Domains: { domains: string[] } };

export type UrlSupport =
  | "Dedicated"
  | { Generic: { priority: number } }
  | "MaybeSupported";

export type UrlSupportMatcherRule = {
  matcher: UrlSupportMatcher;
  support: UrlSupport;
};

export interface PluginSchema {
  name: string;
  description: string | null;
  db: DbSchema | null;
  import_matchers: UrlSupportMatcherRule[];
}

export interface ImportJob {
  url: string;
}

export interface ImportOutput {
  items: Item[];
}

export interface Plugin {
  schema(): PluginSchema;
  fetchUrl(job: FetchUrlJob): Promise<FetchUrlOutput | null>;
  import(job: ImportJob): Promise<ImportOutput | null>;
}

const USERAGENT_CHROME_WINDOWS =
  "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML";

export async function fetchHtmlSuccess(url: string): Promise<string> {
  const response = await fetch(url, {
    headers: {
      "user-agent": USERAGENT_CHROME_WINDOWS,
    },
  });
  if (response.status < 200 || response.status > 299) {
    throw new Error(
      `Request to ${url} failed with status ${response.status}`,
    );
  }
  const body = await response.text();
  return body;
}
