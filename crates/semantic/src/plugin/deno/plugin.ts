
// FIXME: generate interfaces from Rust types

export interface JoinItem<T = Record<string, any>> {
  name: string,
  items: Item<T>[],
}

export interface Item<T = Record<string, any>> {
  data: T,
  joins: JoinItem,

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

export type ImportMatch = 'All' | {Domains: [string]};

export interface PluginSchema {
  name: string;
  description: string | null;
  db: DbSchema|null;
  import_matches: ImportMatch[];
}

export interface Plugin {
  schema(): PluginSchema;
  import(url: string): Promise<ImportOutput|null>;
}
