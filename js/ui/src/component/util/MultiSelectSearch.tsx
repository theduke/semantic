import { createSignal, For, Match, Show, Switch } from "solid-js";
import { JSX } from "solid-js/jsx-runtime";
import { debounce } from "@solid-primitives/scheduled";
import { Control } from "../bulma/form";
import { Icon } from "../bulma/icon";
import { Panel, PanelBlock } from "../bulma/panel";
import { loadAsError, loadAsSuccess, LoadState, SPINNER } from "./load";
import { NotificationError } from "../bulma/notification";

import styles from "./MultiSelectSearch.module.css";

export interface MultiSelectProps<V> {
  defaultItems?: V[];
  initialSelection?: V[];
  search: (term: string, afterItem?: V) => Promise<V[]>;
  searchPlaceholder?: string;

  // Content displayed when no search term was entered yet.
  renderIdle?: () => JSX.Element;
  renderItems?: (items: V[], onSelect: (index: number) => void) => JSX.Element;
  renderItemWrapper?: (items: JSX.Element) => JSX.Element;
  renderItem?: (item: V, onSelect: () => void) => JSX.Element;

  renderSelected?: (items: V[]) => JSX.Element;

  onChange?: (selectedItems: V[]) => void;
}

export function MultiSelectSearch<V>(props: MultiSelectProps<V>): JSX.Element {
  let inputRef: HTMLInputElement | undefined;

  const [searchTerm, setSearchTerm] = createSignal("");

  const initialState: LoadState<V[]> =
    props.defaultItems && props.defaultItems.length > 0
      ? { state: "success", data: props.defaultItems }
      : { state: "idle" };
  const [loader, setLoader] = createSignal<LoadState<V[]>>(initialState);
  const [selected, setSelected] = createSignal<V[]>(
    props.initialSelection ?? []
  );

  const onSearch = debounce((term: string) => {
    props
      .search(term)
      .then((values) => {
        setLoader({ state: "success", data: values });
      })
      .catch((error) => {
        // TODO: better error formatting.
        setLoader({ state: "error", error: error.toString() });
      });
  }, 500);

  const onInput = () => {
    const term = inputRef?.value.trim() ?? "";

    if (term === "") {
      setLoader({ state: "idle" });
      onSearch.clear();
      if (props.defaultItems) {
        setLoader({ state: "success", data: props.defaultItems });
      }
    } else {
      setLoader({ state: "loading" });
      onSearch(term);
    }
  };

  const doSelect = (itemIndex: number) => {
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

  const doUnselect = (itemIndex: number) => {
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

  const renderIdle =
    props.renderIdle ??
    (() => {
      return (
        <PanelBlock>Please enter a search term to start looking.</PanelBlock>
      );
    });

  console.log(styles);

  const choices = (
    <Switch>
      <Match when={loader().state === "idle"}>{renderIdle}</Match>

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
            return props.renderItems(items, doSelect);
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
                // return (
                //   <PanelBlock
                //     class={styles.multiselectOption}
                //     onclick={[doSelect, index()]}
                //   >
                //     {props.renderItem(item)}
                //   </PanelBlock>
                // );
                return renderItem(item, () => doSelect(index()));
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
    selectedRender = props.renderSelected(selected());
  } else {
    selectedRender = (
      <Show when={selected().length > 0}>
        <PanelBlock>
          <hr />
          <strong>Selected:</strong>
        </PanelBlock>

        <For each={selected()}>
          {(item, index) => {
            return (
              <PanelBlock
                class={styles.multiselectSelected}
                onclick={[doUnselect, index()]}
              >
                {
                  // fixme: handle missing renderItem!
                  props.renderItem?.(item, () => doUnselect(index()))
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
            oninput={onInput}
            value={searchTerm()}
            ref={inputRef}
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
