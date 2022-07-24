import { leading, debounce } from "@solid-primitives/scheduled";
import {
  createResource,
  createSignal,
  ErrorBoundary,
  For,
  JSX,
  Match,
  onMount,
  Show,
  Switch,
} from "solid-js";
import { Notification } from "../bulma/notification";
import { SearchInput } from "../bulma/SearchInput";
import { renderError, SPINNER } from "./load";

export interface SearchSelectProps<T> {
  search: (term: string) => Promise<T[]>;
  renderItem: (item: T, index: number, onClick: () => void) => JSX.Element;
  onSelect: (item: T) => void;
  searchPlaceholder?: string;
  onCancel?: () => void;
  noResultsFallback?: () => JSX.Element;
  autoFocus?: boolean;
}

export function SearchSelect<T>(props: SearchSelectProps<T>): JSX.Element {
  const [term, setTerm] = createSignal("");

  const [res] = createResource(term, props.search);

  let inputRef: HTMLInputElement | undefined;

  const onSearch = leading(
    debounce,
    (term: string) => {
      setTerm(term.trim());
    },
    500
  );

  const noResultsFallback =
    props.noResultsFallback ?? (() => <p>Nothing found.</p>);

  console.debug({ f: props.autoFocus });
  if (props.autoFocus) {
    onMount(() => {
      console.log({ inputRef });
      inputRef?.focus();
    });
  }

  return (
    <div>
      <div class="mb-3">
        <SearchInput
          ref={inputRef}
          placeholder={props.searchPlaceholder ?? "Search..."}
          onInput={(e) => onSearch(e.currentTarget.value)}
        />
      </div>

      <div>
        <ErrorBoundary fallback={renderError}>
          <Switch>
            <Match when={term().length < 1}>
              <Notification>Enter a search term to see results.</Notification>
            </Match>
            <Match when={res.loading}>{SPINNER}</Match>
            <Match when={res()}>
              {(items) => (
                <Show when={term().length > 0}>
                  <Show when={items.length > 0} fallback={noResultsFallback}>
                    <For each={res()}>
                      {(item, index) =>
                        props.renderItem(item, index(), () =>
                          props.onSelect(item)
                        )
                      }
                    </For>
                  </Show>
                </Show>
              )}
            </Match>
          </Switch>
        </ErrorBoundary>
      </div>
    </div>
  );
}
