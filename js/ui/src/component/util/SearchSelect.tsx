import { throttle } from "lodash";
import {
  createResource,
  createSignal,
  ErrorBoundary,
  For,
  JSX,
  Match,
  onMount,
  ParentComponent,
  ParentProps,
  Show,
  Switch,
  untrack,
} from "solid-js";
import { Notification } from "../bulma/notification";
import { SearchInput } from "../bulma/SearchInput";
import { renderError, SPINNER } from "./load";

export interface SearchSelectProps<T> {
  search: (term: string) => Promise<T[]>;
  defaultItems?: T[];
  renderItem: (item: T, index: number, onClick: () => void) => JSX.Element;
  itemWrapper?: ParentComponent;
  onSelect: (item: T) => void;
  searchPlaceholder?: string;
  onCancel?: () => void;
  noResultsFallback?: () => JSX.Element;
  autoFocus?: boolean;
}

export function SearchSelect<T>(props: SearchSelectProps<T>): JSX.Element {
  const [term, setTerm] = createSignal("");

  const doSearch = (term: string) => {
    if (term.length > 0) {
      return props.search(term);
    } else {
      return props.defaultItems || [];
    }
  };

  const [res] = createResource(term, doSearch);

  const onTermChange = throttle((term: string) => {
    console.log("termChange", { term });
    setTerm(term.trim());
  }, 500);

  const noResultsFallback =
    props.noResultsFallback ??
    (() => (
      <Show
        when={term()?.length > 0}
        fallback={<Notification>Please enter a search term.</Notification>}
      >
        <Notification>Nothing found.</Notification>
      </Show>
    ));

  const ItemWrapper =
    props.itemWrapper || ((props: ParentProps) => <div>{props.children}</div>);

  return (
    <div>
      <div class="mb-3">
        <SearchInput
          autoFocus={props.autoFocus}
          placeholder={props.searchPlaceholder ?? "Search..."}
          onInput={(e) => {
            e.stopPropagation();
            onTermChange(e.currentTarget.value);
          }}
        />
      </div>

      <div>
        <ErrorBoundary fallback={renderError}>
          <Switch>
            <Match when={res()}>
              {(items) => (
                <Show when={items.length > 0} fallback={noResultsFallback}>
                  <ItemWrapper>
                    <For each={res()}>
                      {(item, index) =>
                        props.renderItem(item, index(), () =>
                          props.onSelect(item)
                        )
                      }
                    </For>
                  </ItemWrapper>
                </Show>
              )}
            </Match>
            <Match when={term().length < 1}>
              <Notification>Enter a search term to see results.</Notification>
            </Match>
            <Match when={res.loading}>{SPINNER}</Match>
          </Switch>
        </ErrorBoundary>
      </div>
    </div>
  );
}
