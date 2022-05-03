import { createSignal, For, Match, Show, Switch } from "solid-js";
import { JSX } from "solid-js/jsx-runtime";
import { createDebounce } from "@solid-primitives/debounce";
import { Control } from "../bulma/form";
import { Icon } from "../bulma/icon";
import { Panel, PanelBlock } from "../bulma/panel";
import { loadAsError, loadAsSuccess, LoadState, SPINNER } from "./load";
import { NotificationError } from "../bulma/notification";

import styles from "./MultiSelect.module.css";

export interface MultiSelectProps<V> {
  searchPlaceholder?: string;
  // Content displayed when no search term was entered yet.
  renderIdle?: () => JSX.Element;

  initialSelection?: V[];

  search: (term: string, afterItem?: V) => Promise<V[]>;
  renderItem: (item: V) => JSX.Element;

  onChange?: (selectedItems: V[]) => void;
}

export function MultiSelectSearch<V>(props: MultiSelectProps<V>): JSX.Element {
  let inputRef: HTMLInputElement | undefined;

  let [searchTerm, setSearchTerm] = createSignal("");
  let [loader, setLoader] = createSignal<LoadState<V[]>>({ state: "idle" });
  const [selected, setSelected] = createSignal<V[]>(
    props.initialSelection ?? []
  );

  const onSearch = createDebounce((term: string) => {
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
    }
  };

  const doUnselect = (itemIndex: number) => {
    const items = selected();
    const item = items[itemIndex];
    if (itemIndex < items.length) {
      const newItems = [...items];
      newItems.splice(itemIndex, 1);
      setSelected(newItems);
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

          return (
            <For each={items}>
              {(item, index) => {
                return (
                  <PanelBlock
                    class={styles.multiselectOption}
                    onclick={[doSelect, index()]}
                  >
                    {props.renderItem(item)}
                  </PanelBlock>
                );
              }}
            </For>
          );
        }}
      </Match>
    </Switch>
  );

  const options = (
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
              {props.renderItem(item)}
            </PanelBlock>
          );
        }}
      </For>
    </Show>
  );

  return (
    <Panel class="pb-2">
      <PanelBlock>
        <Control class="has-icons-left">
          <input
            oninput={onInput}
            ref={inputRef}
            className="input"
            type="text"
            placeholder={props.searchPlaceholder ?? "Search..."}
          />
          <Icon icon="search" isLeft />
        </Control>
      </PanelBlock>

      {choices}
      {options}
    </Panel>
  );
}
