import {
  createEffect,
  createSignal,
  Signal,
  JSX,
  ErrorBoundary,
  Suspense,
  createResource,
  Show,
  ParentProps,
} from "solid-js";
import { NotificationError } from "../bulma/notification";

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
  console.error(error);
  return <NotificationError>{error.toString()}</NotificationError>;
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

export interface BoundarySuspenseLoaderProps<T> {
  load: () => Promise<T>;
  render: (data: T) => JSX.Element;
}

export function BoundarySuspenseLoader<T>(
  props: BoundarySuspenseLoaderProps<T>
): JSX.Element {
  const [data] = createResource(props.load);
  return (
    <ErrorBoundary fallback={renderError}>
      <Suspense fallback={SPINNER}>
        <Show when={data()}>
          {(data) => {
            return props.render(data);
          }}
        </Show>
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
