import { ValueMap } from "semantic/dist/api";
import { FACTOR_TYPE } from "semantic/dist/schema";
import { JSX, Owner, runWithOwner } from "solid-js";
import { createStore, SetStoreFunction, Store } from "solid-js/store";
import { MediaHandle, UiRegistry } from "../../semantic/registry";

export interface PlayerProps {
  items: ValueMap[];
}

export type Seconds = number;
export type ProgressPercent = number;

export interface PlayerItem {
  index: number;
  data: ValueMap;
  rendered?: JSX.Element;
  title: string;

  mediaHandle: MediaHandle | null;
  // Only available if item has a media handle.
  duration?: Seconds;
  progress?: ProgressPercent;
}

interface PlayerStatus {
  items: ValueMap[];

  activeItem: PlayerItem | null;

  muted: boolean;
  cycle: boolean;
  playing: boolean;
  // Millisecond autoplay interval.
  interval: number | null;

  get isPaused(): boolean;
  get itemCount(): number;
  get hasPrev(): boolean;
  get hasNext(): boolean;
}

export class Player {
  public state: Store<PlayerStatus>;
  private setState: SetStoreFunction<PlayerStatus>;

  private registry: UiRegistry;
  private timeoutReference: number | null = null;
  private renderScope: Owner;

  constructor(registry: UiRegistry, items: ValueMap[], renderScope: Owner) {
    this.registry = registry;
    this.renderScope = renderScope;
    const [getter, setter] = createStore<PlayerStatus>({
      items,
      activeItem: null,
      muted: false,
      cycle: true,
      playing: false,
      interval: 5000,

      get isPaused(): boolean {
        return !this.playing;
      },

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
    this.start = this.start.bind(this);
    this.stop = this.stop.bind(this);
    this.togglePlaying = this.togglePlaying.bind(this);
    this.next = this.next.bind(this);
    this.prev = this.prev.bind(this);
    this.stop = this.stop.bind(this);
    this.goto = this.goto.bind(this);
    this.toggleMuted = this.toggleMuted.bind(this);
    this.toggleCycle = this.toggleCycle.bind(this);
  }

  isPlaying(): boolean {
    return !this.state.isPaused;
  }

  appendItems(items: ValueMap[]) {
    this.setState((old) => ({ items: [...old.items, ...items] }));
  }

  replaceItems(items: ValueMap[]) {
    this.setState({ items });
    this.goto(0);
  }

  shuffleItems() {
    this.setState((old) => {
      const items = [...old.items];
      shuffleArray(items);
      return {
        ...old,
        items,
      };
    });
    this.goto(0);
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

  togglePlaying(): void {
    if (this.state.playing) {
      this.stop();
    } else {
      this.start();
    }
  }

  start() {
    if (this.state.playing || this.state.itemCount < 1) {
      return;
    }
    const active = this.state.activeItem;

    this.setState({ playing: true });

    if (active?.mediaHandle) {
      active.mediaHandle.play();
    } else {
      this.next();
    }
  }

  stop() {
    this.setState({ playing: false });
    if (this.timeoutReference) {
      clearTimeout(this.timeoutReference);
      this.timeoutReference = null;
    }
    if (this.state.activeItem?.mediaHandle) {
      this.state.activeItem.mediaHandle.pause();
    }
  }

  next(): void {
    const target = this.state.activeItem ? this.state.activeItem.index + 1 : 0;
    this.goto(target);
  }

  prev(): void {
    const target = this.state.activeItem ? this.state.activeItem.index - 1 : 0;
    this.goto(target);
  }

  renderItem(item: ValueMap): [JSX.Element, MediaHandle | null] {
    const ty = item[FACTOR_TYPE];
    const mediaRenderer = this.registry.mediaRenderer(ty);

    return runWithOwner(this.renderScope, () => {
      let content: JSX.Element | undefined;
      let mediaHandle: MediaHandle | null = null;
      if (mediaRenderer) {
        [mediaHandle, content] = mediaRenderer({
          item,
          showControls: true,
          autoStart: this.state.playing,
          onPaused: () => {
            this.setState({ playing: false });
          },
          onResumed: () => {
            this.setState({ playing: true });
          },
          onFinished: () => {
            this.next();
          },
          onFailed: (_error: string) => {
            // TODO: show error!
            this.next();
          },
          onDurationAvailable: (duration: Seconds) => {
            this.setState((old) => ({
              ...old,
              activeItem: old.activeItem
                ? { ...old.activeItem, duration }
                : null,
            }));
          },
          onProgress: (progress: ProgressPercent) => {
            this.setState((old) => ({
              ...old,
              activeItem: old.activeItem
                ? { ...old.activeItem, progress }
                : null,
            }));
          },
        });
      } else {
        content = this.registry.renderEntity(item, { preview: true });
      }
      return [content, mediaHandle];
    });
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
    const title = this.registry.entityTitle(item);
    const [content, mediaHandle] = this.renderItem(item);

    if (!mediaHandle && this.state.playing && this.state.interval) {
      this.timeoutReference = setTimeout(() => {
        this.next();
      }, this.state.interval) as any as number;
    }

    this.setState({
      activeItem: { index, data: item, title, rendered: content, mediaHandle },
    });
  }
}

function shuffleArray<T>(array: T[]) {
  for (let i = array.length - 1; i > 0; i--) {
    const j = Math.floor(Math.random() * (i + 1));
    [array[i], array[j]] = [array[j], array[i]];
  }
}
