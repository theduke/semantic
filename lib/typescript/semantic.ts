// FIXME: generate interfaces from Rust types

export interface JoinItem<T = Record<string, any>> {
  name: string,
  items: Item<T>[],
}

export interface Item<T = Record<string, any>> {
  data: T,
  joins?: JoinItem[],

}

export interface ImportRelatedUrl {
  label: string,
  url: string,
}

export interface ImportOutput {
  plugin: string,
  items: Item[],
  load_more_url?: string,
  related_urls: ImportRelatedUrl[],
}

export interface DbSchema {

}

export type ImportMatcher = 'All' | {Domains: {domains: [string]}};

export interface ImportMatcherRule {
    matcher: ImportMatcher,
    support: ImportSupport,
}

export type ImportSupport 
  = 'Dedicated'
  | {Generic: {priority: number }}
  | 'MaybeSupported'
  ;

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
