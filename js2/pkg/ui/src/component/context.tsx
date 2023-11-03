import { Context, createContext, useContext } from 'react';
import { UiRegistry } from '../registry';
import { Api } from '@semantic/api/src/api';

export interface Ctx {
  api: Api;
  registry: UiRegistry;
}

export const UiContext: Context<Ctx> = createContext(null as any);

export function useApi(): Api {
  return useContext(UiContext).api;
}

export function useRegistry(): UiRegistry {
  return useContext(UiContext).registry;
}
