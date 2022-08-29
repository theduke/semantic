import {
  Accessor,
  Component,
  createSignal,
  For,
  Match,
  Show,
  Switch,
} from "solid-js";
import { JSX } from "solid-js/jsx-runtime";
import { debounce } from "@solid-primitives/scheduled";
import { Control } from "../bulma/form";
import { Icon } from "../bulma/icon";
import { Panel, PanelBlock } from "../bulma/panel";
import {
  loadAsError,
  loadAsSuccess,
  LoaderView,
  LoadState,
  runWithLoader,
  SPINNER,
} from "./load";
import { NotificationError } from "../bulma/notification";

import styles from "./MultiSelectSearch.module.css";
import { Button, Buttons, IconButton } from "../bulma/button";
import { DeletableTag, Tag, Tags } from "solid-bulma";
import { SearchInput } from "../bulma/SearchInput";

interface SearchPropsSync<V> {
  searchSync: (term: string, selected: V[]) => V[];
}

interface SearchPropsAsync<V> {
  search: (term: string, afterItem?: V) => Promise<V[]>;
}

export interface MultiSelectRenderers<V> {
  renderIdle?: () => JSX.Element;
  renderItems?: (items: V[], onSelect: (index: number) => void) => JSX.Element;
  renderItem?: (item: V, onSelect: () => void) => JSX.Element;
  renderItemWrapper?: (items: JSX.Element) => JSX.Element;
  renderSelected?: (
    items: Accessor<V[]>,
    remove: (itemIndex: number) => void
  ) => JSX.Element;
}

export interface MultiSelectRenderProps<V> extends MultiSelectRenderers<V> {
  searchPlaceholder?: string;

  loader: Accessor<LoadState<V[]>>;
  selected: Accessor<V[]>;
  searchTerm: Accessor<string>;

  onSelect: (itemIndex: number) => void;
  onUnselect: (itemIndex: number) => void;
  onTermChange: (term: string) => void;
}

interface BaseProps<V> extends MultiSelectRenderers<V> {
  defaultItems?: V[];
  initialSelection?: V[];
  searchPlaceholder?: string;

  toggleable?: boolean;

  render?: Component<MultiSelectRenderProps<V>>;

  onChange?: (selectedItems: V[]) => void;
}

type SearchProps<V> = SearchPropsSync<V> | SearchPropsAsync<V>;

type MultiSelectProps<V> = BaseProps<V> & SearchProps<V>;

export function MultiSelectSearch<V>(props: MultiSelectProps<V>): JSX.Element {
  let inputRef: HTMLInputElement | undefined;

  const [searchTerm, setSearchTerm] = createSignal("");

  const initialState: LoadState<V[]> =
    props.defaultItems && props.defaultItems.length > 0
      ? {
          state: "success",
          data: props.defaultItems.filter(
            (x) => !props.initialSelection?.includes(x)
          ),
        }
      : { state: "idle" };
  const [loader, setLoader] = createSignal<LoadState<V[]>>(initialState);
  const [selected, setSelected] = createSignal<V[]>(
    props.initialSelection ?? []
  );

  let onSearch: (term: string) => void;
  let clearSearch: (() => void) | null = null;

  if ("searchSync" in props) {
    onSearch = (term: string) => {
      try {
        const items = props.searchSync(term, selected());
        setLoader({ state: "success", data: items });
      } catch (err: any) {
        setLoader({ state: "error", error: err });
      }
    };
  } else if ("search" in props) {
    const s = debounce((term: string) => {
      runWithLoader(setLoader, props.search(term));
    }, 500);
    onSearch = s;
    clearSearch = () => {
      s.clear();
    };
  }

  const onTermChange = (term: string) => {
    if (term === "") {
      clearSearch?.();
      if (props.defaultItems) {
        setLoader({ state: "success", data: props.defaultItems });
      } else {
        setLoader({ state: "idle" });
      }
    } else {
      setLoader({ state: "loading" });
      onSearch(term);
    }
  };

  const onSelect = (itemIndex: number) => {
    const items = loadAsSuccess(loader()) || [];
    const item = items[itemIndex];
    if (item) {
      const trimmedItems = [...items];
      trimmedItems.splice(itemIndex, 1);
      setLoader({ state: "success", data: trimmedItems });
      setSelected((old) => [...old, item]);
      props.onChange?.(selected());
    }
  };

  const onUnselect = (itemIndex: number) => {
    const items = selected();
    const item = items[itemIndex];
    if (itemIndex < items.length) {
      const newItems = [...items];
      newItems.splice(itemIndex, 1);
      setSelected(newItems);
      props.onChange?.(newItems);
      setLoader((old) => {
        if (old.state === "success") {
          return { state: "success", data: [...old.data, item] };
        } else {
          return old;
        }
      });
    }
  };

  const renderProps: MultiSelectRenderProps<V> = {
    loader: loader,
    selected,
    searchTerm,
    onSelect,
    onUnselect,
    onTermChange,
    renderIdle: props.renderIdle,
    renderItems: props.renderItems,
    renderItem: props.renderItem,
    renderItemWrapper: props.renderItemWrapper,
    renderSelected: props.renderSelected,
  };
  const renderer = props.render ?? defaultRenderer;
  return renderer(renderProps);
}

function defaultRenderer<V>(props: MultiSelectRenderProps<V>): JSX.Element {
  const loader = props.loader;
  const renderIdle =
    props.renderIdle ??
    (() => {
      return (
        <PanelBlock>Please enter a search term to start looking.</PanelBlock>
      );
    });

  const choices = (
    <Switch>
      <Match when={props.loader().state === "idle"}>{renderIdle}</Match>

      <Match when={loader().state === "loading"}>
        <PanelBlock>{SPINNER}</PanelBlock>
      </Match>

      <Match when={loader().state === "error"}>
        <NotificationError>
          An error ocurred:
          <br />
          {loadAsError(loader())}
        </NotificationError>
      </Match>

      <Match when={loader().state === "success"}>
        {() => {
          const items = loadAsSuccess(loader()) ?? [];

          if (items.length === 0) {
            return <PanelBlock>Nothing found...</PanelBlock>;
          }

          if (props.renderItems) {
            return props.renderItems(items, props.onSelect);
          }

          const renderItem = props.renderItem;
          if (!renderItem) {
            throw new Error(
              '"renderItem" is required if "renderItems" is not provided.'
            );
          }
          const renderedItems = (
            <For each={items}>
              {(item, index) => {
                return renderItem(item, () => props.onSelect(index()));
              }}
            </For>
          );

          if (props.renderItemWrapper) {
            return props.renderItemWrapper(renderedItems);
          } else {
            return renderedItems;
          }
        }}
      </Match>
    </Switch>
  );

  let selectedRender: JSX.Element;

  if (props.renderSelected) {
    selectedRender = props.renderSelected(props.selected, props.onUnselect);
  } else {
    selectedRender = (
      <Show when={props.selected().length > 0}>
        <PanelBlock>
          <hr />
          <strong>Selected:</strong>
        </PanelBlock>

        <For each={props.selected()}>
          {(item, index) => {
            return (
              <PanelBlock
                class={styles.multiselectSelected}
                onclick={[props.onUnselect, index()]}
              >
                {
                  // FIXME: handle missing renderItem!
                  props.renderItem?.(item, () => props.onUnselect(index()))
                }
              </PanelBlock>
            );
          }}
        </For>
      </Show>
    );
  }

  return (
    <Panel class="pb-2">
      {selectedRender}

      <PanelBlock>
        <Control class="has-icons-left">
          <input
            onchange={(e) => {
              props.onTermChange(e.currentTarget.value);
            }}
            value={props.searchTerm()}
            class="input"
            type="text"
            placeholder={props.searchPlaceholder ?? "Search..."}
          />
          <Icon icon="search" isLeft />
        </Control>
      </PanelBlock>

      {choices}
    </Panel>
  );
}

export interface MultiSelectViewToggleableTagsProps<V> {
  buildTitle: (item: V) => string;
  searchable?: boolean;
  selectionPlaceholder?: JSX.Element;
}

function buildViewToggleableTags<V>(
  viewProps: MultiSelectViewToggleableTagsProps<V>
): Component<MultiSelectRenderProps<V>> {
  const [isEditing, setIsEditing] = createSignal(false);
  const searchable = viewProps.searchable ?? true;

  return (props: MultiSelectRenderProps<V>): JSX.Element => {
    const selectionFallback = viewProps.selectionPlaceholder ?? (
      <p>Nothing selected yet.</p>
    );

    return (
      <div>
        <div
          class="is-flex"
          style={{
            "justify-content": "flex-start",
            "align-items": "flex-start",
          }}
        >
          <IconButton
            color={isEditing() ? "is-primary" : undefined}
            icon="pencil"
            onclick={() => setIsEditing((old) => !old)}
          />
          <div class="ml-4">
            <Show
              when={props.selected().length > 0}
              fallback={selectionFallback}
            >
              <Tags>
                <For each={props.selected()}>
                  {(item, index) => {
                    const title = viewProps.buildTitle(item);

                    return (
                      <DeletableTag onDelete={() => props.onUnselect(index())}>
                        {title}
                      </DeletableTag>
                    );
                  }}
                </For>
              </Tags>
            </Show>

            <Show when={isEditing()}>
              {() => {
                const search = searchable ? (
                  <div class="mb-4">
                    <SearchInput
                      onInput={(e) => props.onTermChange(e.currentTarget.value)}
                    />
                  </div>
                ) : null;

                return (
                  <div>
                    <hr />
                    {search}

                    <LoaderView loader={props.loader}>
                      {(items) => {
                        return (
                          <Show
                            when={items.length > 0}
                            fallback={<p>Nothing found...</p>}
                          >
                            <Buttons>
                              <For each={items}>
                                {(item, index) => (
                                  <Button
                                    outlined
                                    rounded
                                    size="is-small"
                                    onclick={() => props.onSelect(index())}
                                  >
                                    {viewProps.buildTitle(item)}
                                  </Button>
                                )}
                              </For>
                            </Buttons>
                          </Show>
                        );
                      }}
                    </LoaderView>
                  </div>
                );
              }}
            </Show>
          </div>
        </div>
      </div>
    );
  };
}

export type MultiSelectToggleableTagsProps<V> = {
  initialSelection?: V[];
  buildTitle: (item: V) => string;
  searchable?: boolean;
  defaultItems: V[];
  selectionPlaceholder?: JSX.Element;
  onChange?: (selectedItems: V[]) => void;
} & (SearchPropsSync<V> | SearchPropsAsync<V>);

export function MultiSelectToggleableTags<V>(
  props: MultiSelectToggleableTagsProps<V>
): JSX.Element {
  return (
    <MultiSelectSearch<V>
      defaultItems={props.defaultItems}
      initialSelection={props.initialSelection}
      search={("search" in props ? props.search : undefined) as any}
      searchSync={"searchSync" in props ? props.searchSync : undefined}
      render={buildViewToggleableTags({
        buildTitle: props.buildTitle,
        searchable: props.searchable,
        selectionPlaceholder: props.selectionPlaceholder,
      })}
      onChange={props.onChange}
    />
  );
}
