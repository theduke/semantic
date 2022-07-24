import { Context, createContext, useContext } from "solid-js";
import { Api } from "./api";
import { UiRegistry } from "./semantic/registry";

export const UiRegistryContext: Context<UiRegistry> = createContext(
  new UiRegistry({ db: { attributes: [], entities: [], indexes: [] } })
);

export function useRegistry(): UiRegistry {
  return useContext(UiRegistryContext);
}

export const ApiContext: Context<Api> = createContext(new Api());

export function useApi(): Api {
  return useContext(ApiContext);
}
