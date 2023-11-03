import { ApiError } from '@semantic/api/src/core';
import { ReactNode, useEffect, useMemo, useRef, useState } from 'react';

export type LoadState<T, P = void, E = any> =
  | {
      type: 'idle';
      params: undefined;
      data: undefined;
      oldParams: undefined;
      oldData: undefined;
    }
  | {
      type: 'loading';
      params: P;
      data: undefined;
      oldParams: P | undefined;
      oldData: T | undefined;
    }
  | {
      type: 'loaded';
      params: P;
      data: T;
      oldParams: undefined;
      oldData: undefined;
    }
  | {
      type: 'error';
      params: P;
      data: undefined;
      error: E;
      oldParams: P | undefined;
      oldData: T | undefined;
    };

export type ApiLoadState<T, P = void> = LoadState<T, P, ApiError>;

export type Actions<P> = {
  load: (params: P) => void;
};

export function useLoader<T, P, E = any>(
  load: (params: P) => Promise<T>,
  initialParams?: P | undefined,
  initialData?: T,
): [LoadState<T, P, E>, Actions<P>] {
  const [params, setParams] = useState<P | undefined>(initialParams);
  const [data, setData] = useState<LoadState<T, P, E>>(() => {
    if (initialParams === undefined) {
      return {
        type: 'idle',
        params: undefined,
        data: undefined,
        oldParams: undefined,
        oldData: undefined,
      };
    } else if (initialData) {
      return {
        type: 'loaded',
        params: initialParams,
        data: initialData,
        oldParams: undefined,
        oldData: undefined,
      };
    } else {
      return {
        type: 'loading',
        params: initialParams,
        data: undefined,
        oldParams: undefined,
        oldData: undefined,
      };
    }
  });

  const refs = useRef<{ abort: AbortController | null; shouldLoad: boolean }>({
    abort: null,
    shouldLoad: initialParams !== undefined,
  });

  useEffect(() => {
    // Stop running request.
    refs.current.abort?.abort();
    refs.current.abort = null;

    if (params === undefined) {
      setData({
        type: 'idle',
        params: undefined,
        data: undefined,
        oldParams: undefined,
        oldData: undefined,
      });
    } else if (refs.current.shouldLoad) {
      refs.current.abort = new AbortController();
      refs.current.shouldLoad = false;

      const oldParams = data?.oldParams;
      const oldData = data?.oldData;

      load(params)
        .then((data: T) => {
          setData({
            type: 'loaded',
            params,
            data: data,
            oldParams: undefined,
            oldData: undefined,
          });
        })
        .catch((err) => {
          setData({
            type: 'error',
            params,
            error: err,
            data: undefined,
            oldParams,
            oldData,
          });
        });
    }

    return () => {
      refs.current.abort?.abort();
    };
  }, [params]);

  const actions = useMemo(() => {
    return {
      load: (params: P) => {
        refs.current.shouldLoad = true;
        setParams(params);
      },
      reset: () => {
        setParams(undefined);
      },
    };
  }, []);

  return [data, actions];
}

export interface LoaderViewProps<T, P, E> {
  state: LoadState<T, P, E>;
  render: (data: T) => ReactNode;
  renderError?: (error: any) => ReactNode;
  renderIdle?: () => ReactNode;
}

export function LoaderView<T, P, E>(
  props: LoaderViewProps<T, P, E>,
): ReactNode {
  switch (props.state.type) {
    case 'idle':
      return props.renderIdle?.() ?? null;

    case 'loading':
      return <div>Loading...</div>;

    case 'error':
      if (props.renderError) {
        return props.renderError(props.state.error);
      } else {
        return <div>Error: {(props.state.error as any).toString()}</div>;
      }

    case 'loaded':
      return props.render(props.state.data);
  }
}
