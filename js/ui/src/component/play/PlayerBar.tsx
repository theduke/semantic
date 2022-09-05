import { Accessor, createEffect, JSX, Show, Signal } from "solid-js";
import { Button, ButtonGroup, Buttons, IconButton } from "../bulma/button";
import { Player, PlayerItem } from "./Player";

export interface PlayerBarProps {
  player: Player;
  filterActive: Signal<boolean>;
  onItemButtonClick: (item: PlayerItem) => void;

  isFullscreen?: Accessor<boolean>;
  toggleFullscren?: () => void;
}

export function PlayerBar(props: PlayerBarProps): JSX.Element {
  const p = props.player;
  const state = p.state;

  createEffect(() => {
    console.log({ playing: props.player.state.playing });
  });

  return (
    <div
      style={{
        display: "flex",
        "justify-content": "space-between",
        gap: "0.3rem",
      }}
      class="pl-2 pr-2"
    >
      <div
        style={{
          display: "flex",
          "align-items": "flex-start",
          // "flex-shrink": 0,
          'flex-grow': 1,
          'flex-basis': 0,
        }}
      >
        <ButtonGroup
          attach
          extraClass="mr-2"
          children={[
            <IconButton
              title="Filter"
              icon={"filter"}
              onclick={() => {
                props.filterActive[1]((old) => !old);
              }}
            />,
          ]}
        />

        <ButtonGroup attach>
          {[
            <IconButton
              title={"Mute"}
              icon={state.muted ? "volumeXmark" : "volumeHigh"}
              onclick={p.toggleMuted}
            />,
            <IconButton
              title={"Loop"}
              icon={state.cycle ? "arrowsRotate" : "arrowsLeftRight"}
              onclick={p.toggleCycle}
            />,
            <IconButton
              title={"Shuffle"}
              icon={"shuffle"}
              onclick={() => p.shuffleItems()}
            />,
            props.isFullscreen && props.toggleFullscren !== undefined ? (
              <IconButton
                title={"Fullscreen"}
                icon={props.isFullscreen() ? "minimize" : "maximize"}
                onclick={() => (props as any).toggleFullscren()}
              />
            ) : null,
          ]}
        </ButtonGroup>
      </div>

      <div
        style={{
          display: "flex",
          "flex-shrink": 0,
          "justify-content": "center",
          "align-items": "flex-start",
        }}
      >
        <ButtonGroup attach>
          {[
            <IconButton
              disabled={!state.hasPrev}
              icon={"angleLeft"}
              onclick={p.prev}
            />,
            <IconButton
              disabled={state.itemCount < 1}
              icon={state.playing ? "pause" : "play"}
              onclick={p.togglePlaying}
            />,
            <IconButton
              onclick={p.next}
              disabled={!state.hasNext}
              icon={"angleRight"}
            />,
          ]}
        </ButtonGroup>

        <Show when={state.activeItem}>
          {(item) => (
            <Buttons class="ml-4" attach>
              <Button disabled>{`${item.index + 1}/${state.itemCount}`}</Button>
            </Buttons>
          )}
        </Show>
      </div>

      <div
        style={{
          "flex-shrink": 3,
          'flex-grow': 1,
          display: "flex",
          "overflow-x": "hidden",
          "flex-basis": "0",
          "justify-content": "flex-end",
          "align-items": "flex-start",
        }}
      >
        <ButtonGroup style={{ "max-width": "100%" }}>
          {[
            <Show when={state.activeItem}>
              {(item) => (
                <Button
                  disabled={props.isFullscreen?.()}
                  style={{ "text-overflow": "ellipsis", "max-width": "100%" }}
                  onClick={() => {
                    props.onItemButtonClick(item);
                  }}
                >
                  {item.title}
                </Button>
              )}
            </Show>,
          ]}
        </ButtonGroup>
      </div>
    </div>
  );
}
