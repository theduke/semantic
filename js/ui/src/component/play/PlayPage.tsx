import { JSX } from "solid-js";
import { useRegistry } from "../../context";
import { Player } from "./Player";
import { PlayerBar } from "./PlayerBar";

export function PlayPage(): JSX.Element {
  const reg = useRegistry();

  const player = new Player(reg, []);

  return (
    <div>
      <PlayerBar player={player} />
      player
    </div>
  );
}
