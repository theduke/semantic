import {
  createEffect,
  createResource,
  createSignal,
  For,
  JSX,
  Match,
  Suspense,
  Switch,
} from "solid-js";
import { useApi, useRegistry } from "../../context";
import {
  EntityRenderOpts,
  UiRegistry,
  ValueMap,
} from "../../semantic/registry";
import { Box } from "solid-bulma";
import { renderError, SPINNER } from "../util/load";
import { throttle } from "@solid-primitives/scheduled";
import { NotificationWarning } from "../bulma/notification";
import { isEqual } from "lodash";
import { FACTOR_ID } from "semantic/dist/schema";
import { ToggleableEntityFilter } from "./filter/ToggleableEntityFilter";
import { buildFilterDataSelect, EntityFilter, newFilterData } from "./filter";

// const STORAGE_KEY_BROWSE_PAGE_FILTER_EXPANDED = "browse-page-filter-expanded";

export interface EntityBrowserProps {
  initialFilter?: EntityFilter;
  onFilterChanged?: (filter: EntityFilter) => void;
  storageKey?: string;
}

export function EntityBrowser(props: EntityBrowserProps): JSX.Element {
  const api = useApi();
  const registry = useRegistry();

  const initialFilter = props.initialFilter ?? newFilterData();

  const [filter, setFilter] = createSignal<EntityFilter>(initialFilter);
  const [fetchFilter, setFetchFilter] =
    createSignal<EntityFilter>(initialFilter);

  if (props.onFilterChanged) {
    const handler = props.onFilterChanged;
    createEffect(() => {
      handler(fetchFilter());
    });
  }

  const onFilterChangeDebounced = throttle((filter: EntityFilter) => {
    setFetchFilter(filter);
  }, 500);
  createEffect(() => {
    let newFilter = filter();

    // Ignore sql filter if query is empty.
    if (newFilter.type === "sql" && newFilter.sql.trim() === "") {
      newFilter = newFilterData();
    }

    // Avoid a refetch if filter hasn't changed.
    if (!isEqual(newFilter, fetchFilter())) {
      onFilterChangeDebounced(newFilter);
    }
  });

  const [page, { mutate }] = createResource(fetchFilter, (filter) => {
    if (filter.type === "sql") {
      if (filter.sql.trim()) {
        return api.selectSql(filter.sql);
      } else {
        return [];
      }
    } else {
      const select = buildFilterDataSelect(filter);
      return api.select(select);
    }
  });

  const opts: EntityRenderOpts = {
    preview: true,
    allowDelete: true,
    allowEdit: true,
    onDeleted: (item) => {
      mutate((old) => old?.filter((x) => x[FACTOR_ID] != item[FACTOR_ID]));
    },
  };

  return (
    <div>
      <Box>
        <ToggleableEntityFilter
          initialFilter={initialFilter}
          onChange={setFilter}
          localStorageKey={props.storageKey}
        />
      </Box>

      <div style={{ "max-width": "90%", "min-width": "500px" }}>
        <Suspense fallback={SPINNER}>
          <Switch>
            <Match when={page.loading}>{SPINNER}</Match>
            <Match when={page.error}>{(error) => renderError(error)}</Match>
            <Match when={page()}>
              {(page) => renderItems(registry, page, opts)}
            </Match>
          </Switch>
        </Suspense>
      </div>
    </div>
  );
}

function renderItems(
  reg: UiRegistry,
  items: ValueMap[],
  opts: EntityRenderOpts
): JSX.Element {
  if (items.length < 1) {
    return <NotificationWarning>Nothing found!</NotificationWarning>;
  }

  return (
    <div class="is-flex is-flex-direction-column mb-4" style={{ gap: "2rem" }}>
      <For each={items}>{(item) => reg.renderEntity(item, opts)}</For>
    </div>
  );
}
