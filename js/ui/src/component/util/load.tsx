import {
  createEffect,
  createResource,
  createSignal,
  Signal,
  JSX,
  ErrorBoundary,
  Suspense,
  ParentProps,
  Resource,
  Switch,
  Match,
  ResourceFetcher,
  ResourceOptions,
  ResourceReturn,
} from "solid-js";
import { createStore } from "solid-js/store";
import {
  ResourceActions,
  ResourceSource,
} from "solid-js/types/reactive/signal";
import { setErrorMap } from "zod";
import { NotificationError } from "../bulma/notification";

export type FallibleResource<O> = Resource<O> & { caughtError?: any };

export type FallibleResourceReturn<
  T,
  O extends ResourceOptions<T | undefined> | undefined,
  K = T
> = [
  FallibleResource<
    O extends undefined | null
      ? T | undefined
      : NonNullable<O>["initialValue"] extends undefined
      ? T | undefined
      : T
  >,
  ResourceActions<K>
];

export function createFallibleResource<T, S = true>(
  fetcher: ResourceFetcher<S, T>,
  options?: ResourceOptions<undefined>
): FallibleResourceReturn<T | undefined, typeof options>;
export function createFallibleResource<T, S = true>(
  fetcher: ResourceFetcher<S, T>,
  options: ResourceOptions<T>
): FallibleResourceReturn<T, typeof options>;
export function createFallibleResource<T, S>(
  source: ResourceSource<S>,
  fetcher: ResourceFetcher<S, T>,
  options?: ResourceOptions<undefined>
): FallibleResourceReturn<T | undefined, typeof options>;
export function createFallibleResource<T, S>(
  source: ResourceSource<S>,
  fetcher: ResourceFetcher<S, T>,
  options: ResourceOptions<T>
): FallibleResourceReturn<T, typeof options>;

export function createFallibleResource<T, S>(
  source: ResourceSource<S> | ResourceFetcher<S, T>,
  fetcher?:
    | ResourceFetcher<S, T>
    | ResourceOptions<T>
    | ResourceOptions<undefined>,
  options?: ResourceOptions<T> | ResourceOptions<undefined>
): FallibleResourceReturn<T | undefined, typeof options> {
  if (arguments.length === 2) {
    if (typeof fetcher === "object") {
      options = fetcher as ResourceOptions<T> | ResourceOptions<undefined>;
      fetcher = source as ResourceFetcher<S, T>;
      source = true as ResourceSource<S>;
    }
  } else if (arguments.length === 1) {
    fetcher = source as ResourceFetcher<S, T>;
    source = true as ResourceSource<S>;
  }
  options || (options = {});

  const [errorStore, setErrorStore] = createStore<{ error?: any }>({});

  const wrappedFetcher = async function () {
    try {
      let output = (fetcher as any)(arguments);
      if (typeof output === "object" && "then" in output) {
        output = await output;
      }
      return output;
    } catch (error: any) {
      setErrorStore({ error });
      return undefined;
    }
  };

  const xSource = source as any;
  const res: any =
    xSource === true
      ? createResource(wrappedFetcher as any, options)
      : createResource(source as any, wrappedFetcher as any, options);
  const getter = res[0];

  const wrappedGetter = function () {
    return getter();
  };
  wrappedGetter.error = errorStore.error;
  wrappedGetter.caughtError = errorStore.error;
  res[0] = wrappedGetter;
  return res;
}

export type LoadState<T> =
  | { state: "idle" }
  | { state: "loading" }
  | { state: "error"; error: string }
  | { state: "success"; data: T };

export const IDLE = { state: "idle" };
export const LOADING = { state: "loading" };

export function mkIdle<T>(): LoadState<T> {
  return { state: "idle" };
}

export function mkError<T>(error: any): LoadState<T> {
  return { state: "error", error: error.toString() };
}

export function mkSuccess<T>(data: T): LoadState<T> {
  return { state: "success", data };
}

export function loadAsError<T>(state: LoadState<T>): string | null {
  return state.state === "error" ? state.error : null;
}

export function loadAsSuccess<T>(state: LoadState<T>): T | null {
  return state.state === "success" ? state.data : null;
}

export function startLoader<T>(
  [_, set]: Signal<LoadState<T>>,
  fetcher: () => Promise<T>
): Promise<LoadState<T>> {
  set({ state: "loading" });
  return fetcher()
    .then((data) => {
      const state: LoadState<T> = { state: "success", data: data };
      set(state);
      return state;
    })
    .catch((err) => {
      const state: LoadState<T> = { state: "error", error: err };
      set(state);
      return state;
    });
}

export function createLoader<T>(
  initial: LoadState<T> = { state: "idle" }
): Signal<LoadState<T>> {
  return createSignal<LoadState<T>>(initial);
}

export function renderError(error: any): JSX.Element {
  console.trace(error);
  return (
    <NotificationError>{error.toString() || "Unknown Error"}</NotificationError>
  );
}

export const SPINNER = (
  <div>
    <button class="button is-large is-disabled is-loading" />
  </div>
);

export function spawnLoader<T>(load: () => Promise<T>): Signal<LoadState<T>> {
  const [get, set] = createLoader<T>({ state: "loading" });
  createEffect(() => {
    load()
      .then((data) => set({ state: "success", data: data }))
      .catch((err) => {
        set({
          state: "error",
          // TODO: better error handling
          error: err.toString(),
        });
      });
  });

  return [get, set];
}

export function SuspsenseSpinner(props: ParentProps): JSX.Element {
  return <Suspense fallback={SPINNER}>{props.children}</Suspense>;
}

export type FallibleResourceProps<T> = {
  resource: FallibleResource<T>;
  children: (data: T) => JSX.Element;
};

export function FallibleResource<T>(
  props: FallibleResourceProps<T>
): JSX.Element {
  return (
    <Switch>
      <Match when={props.resource.loading}>{SPINNER}</Match>
      <Match when={props.resource.caughtError}>
        {renderError(props.resource.caughtError)}
      </Match>
      <Match when={props.resource()}>{(data) => props.children(data)}</Match>
    </Switch>
  );
}

export type FallibleResourceLoaderProps<T> = {
  load: () => Promise<T>;
  children: (data: T, actions: ResourceActions<T>) => JSX.Element;
};

export function FallibleResourceLoader<T>(
  props: FallibleResourceLoaderProps<T>
): JSX.Element {
  const [data, actions] = createFallibleResource(props.load);
  return (
    <Switch>
      <Match when={data.loading}>{SPINNER}</Match>
      <Match when={data.caughtError}>{renderError(data.caughtError)}</Match>
      <Match when={data()}>
        {(data) => props.children(data, actions as any)}
      </Match>
    </Switch>
  );
}

export type BoundarySuspenseLoaderProps<T> = {
  load: () => Promise<T>;
  render: (data: T) => JSX.Element;
};

export function BoundarySuspenseLoader<T>(
  props: BoundarySuspenseLoaderProps<T>
): JSX.Element {
  const [data] = createFallibleResource(props.load);
  return (
    <ErrorBoundary fallback={renderError}>
      <Suspense fallback={SPINNER}>
        {() => {
          const out = data();
          return out ? props.render(out) : null;
        }}
      </Suspense>
    </ErrorBoundary>
  );
}

export function BoundarySuspense(props: ParentProps): JSX.Element {
  return (
    <ErrorBoundary fallback={renderError}>
      <Suspense fallback={SPINNER}>{props.children}</Suspense>
    </ErrorBoundary>
  );
}

export interface ResourceViewerProps<T> {
  resource: Resource<T>;
  children: (data: T) => JSX.Element;
  fallback?: JSX.Element;
}

export function ResourceViewer<T>(props: ResourceViewerProps<T>): JSX.Element {
  return (
    <ErrorBoundary fallback={renderError}>
      <Switch fallback={props.fallback}>
        <Match when={props.resource.loading}>{SPINNER}</Match>
        <Match when={props.resource.error}>
          {(error) => <NotificationError>{error.toString()}</NotificationError>}
        </Match>
        <Match when={props.resource()}>{(data) => props.children(data)}</Match>
      </Switch>
    </ErrorBoundary>
  );
}
