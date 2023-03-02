import { Api, ValueMap, newSelect } from "semantic/dist/api";
import { Expr, Select } from "semantic/dist/core";
import { exprAnd, exprAttr, exprIn, exprList, exprLiteral, exprNot } from "semantic/dist/db";
import {
  FACTOR_ID,
  FACTOR_TYPE,
  SEMANTIC_PARENT,
  TY_SEMANTIC_AUDIO,
  TY_SEMANTIC_IMAGE,
  TY_SEMANTIC_VIDEO,
} from "semantic/dist/schema";
import { Box } from "solid-bulma";
import {
  createSignal,
  getOwner,
  JSX,
  Match,
  onCleanup,
  onMount,
  Show,
  Switch,
} from "solid-js";
import { Portal } from "solid-js/web";
import { useApi, useRegistry } from "../../context";
import { Button, Buttons } from "../bulma/button";
import { FieldHorizontal } from "../bulma/form";
import { Modal } from "../bulma/modal";
import { EntityFilter } from "../entity/filter";
import { EntityFilterForm, loadFilter } from "../entity/filter/EntityFilter";
import {
  createLoader,
  loadAsError,
  renderError,
  SPINNER,
  startLoader,
} from "../util/load";
import { Player, PlayerItem } from "./Player";
import { PlayerBar } from "./PlayerBar";

export function PlayPage(): JSX.Element {
  const reg = useRegistry();
  const api = useApi();

  const owner = getOwner();
  if (!owner) {
    throw new Error("no owner found");
  }

  const player = new Player(reg, [], owner);

  const filterActiveSignal = createSignal(false);
  const [filterActive, setFilterActive] = filterActiveSignal;

  let filter: EntityFilter = {
    type: "data",
    searchTerm: "",
    entityTypes: [TY_SEMANTIC_AUDIO, TY_SEMANTIC_VIDEO, TY_SEMANTIC_IMAGE],
    limit: 100_000,
  };
  const [_filterChanged, setFilterChanged] = createSignal(false);
  // const [filter, setFilter] = createSignal<EntityFilter>(emptyEntityFilter());

  let wrapperDiv: HTMLDivElement | undefined;

  const loaderSignal = createLoader<boolean>();
  const [loader, setLoader] = loaderSignal;
  const [isFullscreen, setFullscreen] = createSignal(false);

  const toggleFullscreen = () => {
    if (isFullscreen()) {
      document.exitFullscreen();
      setFullscreen(false);
    } else if (wrapperDiv) {
      wrapperDiv.requestFullscreen();
    }
  };

  const [entityModalItem, setEntityModalItem] = createSignal<ValueMap | null>(
    null
  );

  const [expandToMedia, setExpandToMedia] = createSignal(false);

  const builderExtra = (
    <FieldHorizontal smallLabel label='Expand to media items'>
      <label class="checkbox">
        <input
          type="checkbox"
          onchange={(e) => {
            const selected = e.currentTarget.checked;
            setExpandToMedia(selected);
          }}
        />
        {" "}
        {"Yes/No"}
      </label>
    </FieldHorizontal>
  );

  const onFilterChange = (newFilter: EntityFilter) => {
    filter = newFilter;
    setFilterChanged(true);
  };

  const doLoadFilter = (replace: boolean) => {
    if (loader().state === "loading") {
      return;
    }
    startLoader(loaderSignal, async () => {
      const items = await load(api, filter, { expandToMedia: expandToMedia() });
      if (replace) {
        player.replaceItems(items);
      } else {
        player.appendItems(items);
      }
      setFilterChanged(false);
      setFilterActive(false);
      return true;
    });
  };

  doLoadFilter(true);

  let wasPlaying = false;
  const showItemModal = (item: PlayerItem) => {
    wasPlaying = player.isPlaying();
    setEntityModalItem(item.data);
    player.stop();
  };
  const hideItemModal = () => {
    if (wasPlaying) {
      player.start();
    }
    setEntityModalItem(null);
  };

  const onKeyDown = (e: KeyboardEvent) => {
    // Don't handle any keypresses if modal is open.
    if (entityModalItem() || filterActive()) {
      return;
    }

    switch (e.key) {
      case "ArrowLeft":
        player.prev();
        break;
      case "ArrowRight":
        player.next();
        break;
      case " ":
        player.togglePlaying();
        break;
      case "f":
        toggleFullscreen();
        break;
      case "Escape":
        if (isFullscreen()) {
          toggleFullscreen();
        }
        break;
      default:
    }
  };

  onMount(() => {
    document.addEventListener("keydown", onKeyDown);
  });
  onCleanup(() => {
    document.removeEventListener("keydown", onKeyDown);
  });

  return (
    <div
      ref={wrapperDiv}
      onfullscreenchange={() => {
        console.log("fullscreenchange");
        setFullscreen(document.fullscreenElement !== null);
      }}
      style={{
        display: "flex",
        "flex-direction": "column",
        "flex-grow": 1,
        overflow: "hidden",
      }}
    >
      <Show when={entityModalItem()} keyed>
        {(item) => {
          const content = reg.renderEditableEntity(
            item,
            {
              preview: false,
            },
            (newItem) => { }
          );
          return (
            <Portal>
              <Modal onClose={hideItemModal}>{content}</Modal>
            </Portal>
          );
        }}
      </Show>

      <PlayerBar
        player={player}
        filterActive={filterActiveSignal}
        onItemButtonClick={showItemModal}
        isFullscreen={isFullscreen}
        toggleFullscren={toggleFullscreen}
      />

      <div
        style={{
          display: "flex",
          "flex-grow": 1,
          overflow: "hidden",
          "justify-content": "center",
          "align-items": "center",
        }}
      >
        <Switch>
          <Match when={filterActive()}>
            <div style={{ 'overflow-y': 'scroll', height: '100%' }}>
              <Box>
                <EntityFilterForm
                  initialFilter={filter}
                  onChange={onFilterChange}
                  builderExtra={builderExtra}
                />

                <hr />

                <Buttons class="mt-3">
                  <Button
                    loading={loader().state === "loading"}
                    onclick={() => doLoadFilter(true)}
                  >
                    Replace
                  </Button>
                  <Button
                    loading={loader().state === "loading"}
                    onclick={() => doLoadFilter(false)}
                  >
                    Add
                  </Button>
                </Buttons>
              </Box>
            </div>
          </Match>

          <Match when={loader().state === "loading"}>{SPINNER}</Match>

          <Match when={loadAsError(loader())} keyed>
            {(err) => renderError(err)}
          </Match>

          <Match when={player.state.activeItem} keyed>
            {(item) => {
              return item.rendered;
            }}
          </Match>
        </Switch>
      </div>
    </div>
  );
}

interface LoadOptions {
  expandToMedia: boolean;
}

export async function load(
  api: Api,
  filter: EntityFilter,
  options: LoadOptions,
): Promise<ValueMap[]> {
  const items = await loadFilter(api, filter);

  if (options.expandToMedia) {
    return loadExpand(api, items);
  } else {
    return items;
  }
}

async function loadExpand(api: Api, items: ValueMap[], ignoredIds?: Expr[]): Promise<ValueMap[]> {
  const mediaItems: ValueMap[] = [];
  const expandIds: Expr[] = [];

  ignoredIds = ignoredIds ?? [];

  for (const item of items) {
    let id: string | undefined;

    switch (item[FACTOR_TYPE]) {
      case TY_SEMANTIC_IMAGE:
      case TY_SEMANTIC_VIDEO:
      case TY_SEMANTIC_AUDIO:
        id = item[FACTOR_ID];
        if (id) {
          ignoredIds.push(exprLiteral(id));
        }
        mediaItems.push(item);
        break;
      default:
        id = item[FACTOR_ID];
        if (id) {
          expandIds.push(exprLiteral(id));
        }
    }
  }

  const query: Select = {
    ...newSelect(),
    filter: exprAnd(
      exprIn(exprAttr(SEMANTIC_PARENT), exprList(expandIds)),
      exprNot(exprIn(exprAttr(FACTOR_ID), exprList(ignoredIds))),
    ),
    // TODO: no any
    limit: 1_000 as any,
  };

  console.debug('expanding fetched items', { expandQuery: query });
  const newItems = await api.select(query);
  console.debug('expanded fetched items', { count: newItems.length })

  if (newItems.length > 0) {
    const recurseNewItems = await loadExpand(api, newItems, ignoredIds);
    return [...mediaItems, ...recurseNewItems];
  } else {
    return mediaItems;
  }
}
