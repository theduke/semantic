import { ValueMap } from "semantic/dist/api";
import {
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
  };
  const [filterChanged, setFilterChanged] = createSignal(false);

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

  const onFilterChange = (newFilter: EntityFilter) => {
    filter = newFilter;
    setFilterChanged(true);
  };
  const doLoadFilter = (replace: boolean) => {
    if (loader().state === "loading") {
      return;
    }
    startLoader(loaderSignal, async () => {
      const items = await loadFilter(api, filter);
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
      <Show when={entityModalItem()}>
        {(item) => {
          const content = reg.renderEditableEntity(
            item,
            {
              preview: false,
            },
            (newItem) => {}
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
            <Box>
              <EntityFilterForm
                initialFilter={filter}
                onChange={onFilterChange}
              />

              <Buttons>
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
          </Match>

          <Match when={loader().state === "loading"}>{SPINNER}</Match>

          <Match when={loadAsError(loader())}>
            {(err) => renderError(err)}
          </Match>

          <Match when={player.state.activeItem}>
            {(item) => {
              return item.rendered;
            }}
          </Match>
        </Switch>
      </div>
    </div>
  );
}
