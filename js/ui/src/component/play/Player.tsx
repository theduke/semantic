import { ValueMap } from "semantic/dist/api";
import { JSX, Signal } from "solid-js";
import {
  createStore,
  SetStoreFunction,
  Store,
  StoreSetter,
} from "solid-js/store";
import { MediaHandle, UiRegistry } from "../../semantic/registry";

export interface PlayerProps {
  items: ValueMap[];
}

interface PlayerItem {
  index: number;
  data: ValueMap;
  mediaHandle?: MediaHandle;
  rendered?: JSX.Element;
}

interface PlayerStatus {
  items: ValueMap[];

  activeItem: PlayerItem | null;

  muted: boolean;
  cycle: boolean;
  playing: boolean;
  // Millisecond autoplay interval.
  interval: number | null;

  get itemCount(): number;
  get hasPrev(): boolean;
  get hasNext(): boolean;
}

export class Player {
  public state: Store<PlayerStatus>;
  private setState: SetStoreFunction<PlayerStatus>;

  private registry: UiRegistry;
  private timeoutReference: number | null = null;

  constructor(registry: UiRegistry, items: ValueMap[]) {
    this.registry = registry;
    const [getter, setter] = createStore<PlayerStatus>({
      items,
      activeItem: null,
      muted: false,
      cycle: true,
      playing: true,
      interval: null,

      get itemCount(): number {
        return this.items.length;
      },

      get hasPrev(): boolean {
        return (this.activeItem && this.activeItem.index > 0) ?? false;
      },

      get hasNext(): boolean {
        return (
          (this.activeItem && this.activeItem.index < this.itemCount - 1) ??
          false
        );
      },
    });
    this.state = getter;
    this.setState = setter;

    this.setMuted = this.setMuted.bind(this);
    this.setCycle = this.setCycle.bind(this);
    this.setInterval = this.setInterval.bind(this);
    this.setPlaying = this.setPlaying.bind(this);
    this.togglePlaying = this.togglePlaying.bind(this);
    this.next = this.next.bind(this);
    this.prev = this.prev.bind(this);
    this.stop = this.stop.bind(this);
    this.goto = this.goto.bind(this);
    this.toggleMuted = this.toggleMuted.bind(this);
    this.toggleCycle = this.toggleCycle.bind(this);
  }

  setMuted(muted: boolean): void {
    this.setState({ muted });
  }

  toggleMuted(): void {
    this.setState((old) => ({ muted: !old.muted }));
  }

  setCycle(cycle: boolean): void {
    this.setState({ cycle });
  }

  toggleCycle(): void {
    this.setState((old) => ({ cycle: !old.cycle }));
  }

  setInterval(interval: number | null): void {
    this.setState({ interval });
  }

  setPlaying(playing: boolean): void {
    this.setState({ playing });
  }

  togglePlaying(): void {
    this.setState((old) => ({ playing: !old.playing }));
  }

  next(): void {
    const target = this.state.activeItem ? this.state.activeItem.index + 1 : 0;
    this.goto(target);
  }

  prev(): void {
    const target = this.state.activeItem ? this.state.activeItem.index - 1 : 0;
    this.goto(target);
  }

  stop() {
    this.setState({ playing: false });
    if (this.timeoutReference) {
      clearTimeout(this.timeoutReference);
      this.timeoutReference = null;
    }
  }

  goto(index: number): void {
    if (this.timeoutReference) {
      clearTimeout(this.timeoutReference);
      this.timeoutReference = null;
    }

    const count = this.state.itemCount;
    const cycle = this.state.cycle;

    if (count === 0) {
      return;
    } else if (index >= count) {
      if (cycle) {
        index = 0;
      } else {
        this.stop();
      }
    } else if (index < 0) {
      if (cycle) {
        index = count - 1;
      } else {
        this.stop();
      }
    }

    const item = this.state.items[index];

    this.setState({ activeItem: { index, data: item } });
  }
}
