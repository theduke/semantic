import { Api } from '@semantic/api/src/api';
import { PropsWithChildren, ReactNode, useRef } from 'react';
import { useLoader } from './loader';
import { UiContext } from './context';
import { UiRegistry } from '../registry';
import { SemanticSchema } from '@semantic/api/src/core';

import { basePlugin } from '../base';
import { MantineProvider } from '@mantine/core';

async function tryLogin(api: Api): Promise<SemanticSchema> {
  try {
    const status = await api.serverStatus();
    if (!status.backend_initialized) {
      throw new Error('Backend not initialized');
    }
    const schema = await api.schema();
    return schema;
  } catch (err: any) {
    console.error(err);
    throw err;
  }
}

export function LoginWrapper(props: PropsWithChildren): ReactNode {
  const api = useRef<Api>(new Api('http://localhost:3000'));
  const registry = useRef<UiRegistry>();

  const [status, _actions] = useLoader(async () => {
    const data = await tryLogin(api.current);
    registry.current = new UiRegistry(data);
    registry.current.registerPlugin(basePlugin());

    console.debug('initial schema loaded');
    return data;
  }, null);

  switch (status.type) {
    case 'idle':
      return 'Idle';

    case 'loading':
      return 'Loading...';

    case 'error':
      return <div>{status.error.toString()}</div>;

    case 'loaded':
      return (
        <UiContext.Provider
          value={{ api: api.current, registry: registry.current! }}
        >
	  <MantineProvider>
            {props.children}
	  </MantineProvider>
        </UiContext.Provider>
      );
  }
}

