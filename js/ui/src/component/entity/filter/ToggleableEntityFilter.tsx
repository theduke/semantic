import { createSignal, JSX, Show, splitProps } from "solid-js";
import { setAttribute } from "solid-js/web";
import { defineConfig } from "vite";
import { Button } from "../../bulma/button";
import { Icon } from "../../bulma/icon";
import { EntityFilter } from "./EntityFilter";

export interface ToggleableEntityFilterProps {
  initialFilter?: EntityFilter;
  onChange: (filter: EntityFilter) => void;

  // Whether to show the filter by default.
  // NOTE: superseded by a false setting via localStorageKey.
  defaultExpanded?: boolean;

  // Key in localStorage that persists if the filter is expanded or not.
  localStorageKey?: string;
}

function storageGetExpanded(key: string, defaultExpanded: boolean): boolean {
  const value = localStorage.getItem(key);
  if (value === "1") {
    return true;
  } else if (value === "0") {
    return false;
  } else {
    return defaultExpanded;
  }
}

export function ToggleableEntityFilter(
  props: ToggleableEntityFilterProps
): JSX.Element {
  const [local, rest] = splitProps(props, [
    "localStorageKey",
    "defaultExpanded",
  ]);
  const filter = <EntityFilter {...rest} />;

  const initialExpanded = local.localStorageKey
    ? storageGetExpanded(local.localStorageKey, local.defaultExpanded ?? false)
    : local.defaultExpanded ?? false;
  const [active, setActive] = createSignal(initialExpanded);

  const toggleActive = () => {
    setActive(true);
    if (local.localStorageKey) {
      localStorage.setItem(local.localStorageKey, "1");
    }
  };

  const toggleInactive = () => {
    setActive(false);
    if (local.localStorageKey) {
      localStorage.setItem(local.localStorageKey, "0");
    }
  };

  const toggle = (
    <div class="mb-2">
      <Button onclick={toggleActive} title="Show filter">
        <Icon icon="filter" />
      </Button>
    </div>
  );

  return (
    <div>
      <Show when={active()} fallback={toggle}>
        <div class="mb-2">
          <Button color="is-info" onclick={toggleInactive} title="Hide filter">
            <Icon icon="filter" />
          </Button>
        </div>

        {filter}
      </Show>
    </div>
  );
}
