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
import { GenericPage } from "../util";
import { renderError, SPINNER } from "../util/load";
import { EntityFilter } from "./filter/EntityFilter";
import {
  buildFilterDataSelect,
  newFilterData,
} from "./filter/EntityFilterForm";
import { throttle } from "@solid-primitives/scheduled";
import { NotificationWarning } from "../bulma/notification";
import { isEqual } from "lodash";

export function BrowsePage(): JSX.Element {
  const api = useApi();
  const registry = useRegistry();

  const [filter, setFilter] = createSignal<EntityFilter>(newFilterData());
  const [fetchFilter, setFetchFilter] = createSignal<EntityFilter>(
    newFilterData()
  );

  // const onFilterChangeDebounced = leading(debounce, (filter: EntityFilter) => {
  //   setFetchFilter(filter);
  // }, 500);
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

  const [page] = createResource(fetchFilter, (filter) => {
    console.debug("filter changed", { filter });
    if (filter.type === "sql") {
      if (filter.sql.trim()) {
        return api.selectSql(filter.sql);
      } else {
        return [];
      }
    } else {
      const select = buildFilterDataSelect(filter);
      console.debug({ select });
      return api.select(select);
    }
  });

  const opts: EntityRenderOpts = {
    preview: true,
  };

  return (
    <GenericPage title="Browse">
      <Box>
        <EntityFilter onChange={setFilter} />
      </Box>

      <Suspense fallback={SPINNER}>
        <Switch>
          <Match when={page.loading}>{SPINNER}</Match>
          <Match when={page.error}>{(error) => renderError(error)}</Match>
          <Match when={page()}>
            {(page) => renderItems(registry, page, opts)}
          </Match>
        </Switch>
      </Suspense>
    </GenericPage>
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
