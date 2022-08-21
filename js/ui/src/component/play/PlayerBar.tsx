import { JSX, Show } from "solid-js";
import { Button, Buttons, IconButton } from "../bulma/button";
import { Player } from "./Player";

export interface PlayerBarProps {
  player: Player;
}

export function PlayerBar(props: PlayerBarProps): JSX.Element {
  const p = props.player;
  const state = p.state;

  return (
    <div>
      <Buttons>
        <IconButton
          disabled={!state.hasPrev}
          icon={"angleLeft"}
          onclick={p.next}
        />
        <IconButton
          disabled={state.itemCount < 1}
          icon={state.playing ? "play" : "pause"}
          onclick={p.togglePlaying}
        />
        <IconButton disabled={!state.hasNext} icon={"angleRight"} />

        <Show when={state.activeItem}>
          {(item) => (
            <Button disabled>{`${item.index + 1}/${state.itemCount}`}</Button>
          )}
        </Show>
      </Buttons>

      <Buttons>
        <Show when={state.activeItem}>
          {(item) => <Button>{item.title}</Button>}
        </Show>
      </Buttons>

      <Buttons>
        <IconButton
          title={"Mute"}
          icon={state.muted ? "volumeXmark" : "volumeHigh"}
          onclick={p.toggleMuted}
        />
        <IconButton
          title={"Loop"}
          icon={state.cycle ? "arrowsRotate" : "arrowsLeftRight"}
          onclick={p.toggleCycle}
        ></IconButton>
      </Buttons>
    </div>
  );
}
