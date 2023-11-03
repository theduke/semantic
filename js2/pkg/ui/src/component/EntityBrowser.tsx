import { ReactNode } from 'react';
import { useLoader } from './loader';
import { useApi, useRegistry } from './context';
import { Api, newSelect } from '@semantic/api/src/api';
import {  ValueMap } from '@semantic/api';
import { Stack } from '@mantine/core';

export interface QueryParams {}

export function EntityBrowser(): ReactNode {
  console.debug('EntityBrowser');
  const api = useApi();
  const registry = useRegistry();

  const [status, _actions] = useLoader((p: QueryParams) => load(api, p), {});


  let itemContent: ReactNode;
  switch (status.type) {
    case 'idle':
      itemContent = null;
      break;

    case 'loading':
      itemContent = 'Loading...';
      break;

    case 'error':
      itemContent = <div>ERROR: {status.error.toString()}</div>;
      break;

    case 'loaded':
      if (status.data.length === 0) {
        itemContent = <div>Nothing found...</div>;
      } else {
        const renderOpts = {
          preview: true,
        };

        const items = status.data.map((item) => {
          return registry.renderEntity(item, renderOpts);
        });

        itemContent = <Stack>{items}</Stack>;
      }
  }

  return (
      <div>{itemContent}</div>
  );
}

async function load(api: Api, _params: QueryParams): Promise<ValueMap[]> {
  const sel = newSelect();
  const items = await api.select(sel);
  return items;
}
