import { Context, createContext, useContext } from "solid-js";
import { Api } from "semantic/dist/api";
import { UiRegistry } from "./semantic/registry";
import { SemanticSchema } from "semantic/dist/core";

export const UiRegistryContext: Context<UiRegistry> = createContext(
  new UiRegistry({ db: { attributes: [], classes: [], indexes: [] } })
);

export function useRegistry(): UiRegistry {
  return useContext(UiRegistryContext);
}

export const ApiContext: Context<Api> = createContext(new Api("/"));

export function useApi(): Api {
  return useContext(ApiContext);
}

const STORAGE_KEY_SCHEMA = "_semantic_schema";

export function storageSaveSchema(schema: SemanticSchema) {
  localStorage.setItem(STORAGE_KEY_SCHEMA, JSON.stringify(schema));
}

export function storageLoadSchema(): SemanticSchema | null {
  const item = localStorage.getItem(STORAGE_KEY_SCHEMA);
  if (!item) {
    return null;
  }
  try {
    const data = JSON.parse(item);
    // TODO: validation!
    return data;
  } catch {
    return null;
  }
}
