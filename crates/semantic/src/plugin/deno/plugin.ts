
// FIXME: generate interfaces from Rust types

export interface JoinItem<T = DataMap> {
  name: string,
  items: Item<T>[],
}

export type DataMap = Record<string, any>;

export interface Item<T = DataMap> {
  data: T,
  joins: JoinItem,
}

export interface ImportItem<T = DataMap> {
  data: T,
  joins: ImportJoinItem[],
  import_requires_fetch: boolean;
}

export interface ImportJoinItem<T = DataMap> {
  name: string,
  items: ImportItem<T>[],
}

export interface ImportRelatedUrl {
  label: string,
  url: string,
}

export interface ImportOutput {
  items: ImportItem[],
  load_more_url?: string,
  related_urls: ImportRelatedUrl[],
}

export interface DbSchema {

}

export type ImportMatcher = 'All' | {Domains: {domains: string[]}};

export type ImportSupport
  = 'Dedicated'
  | { Generic: {priority: number} }
  | 'MaybeSupported';

export type ImportMatcherRule = {
  matcher: ImportMatcher;
  support: ImportSupport;
};

export interface PluginSchema {
  name: string;
  description: string | null;
  db: DbSchema|null;
  import_matchers: ImportMatcherRule[];
}

export interface Plugin {
  schema(): PluginSchema;
  import(url: string): Promise<ImportOutput|null>;
}
