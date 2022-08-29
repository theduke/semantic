import { createEffect, createSignal, JSX, Match, Switch } from "solid-js";
import { Tabs } from "../../bulma/tabs";
import { FilterBuilder } from "./EntityFilterForm";
import { EntityFilterSqlForm } from "./EntityFilterSqlForm";
import { Api, ValueMap } from "semantic/dist/api";

import {
  buildFilterDataSelect,
  EntityFilter,
  EntityFilterData,
  EntityFilterSql,
  filterIsData,
  filterIsSql,
  newFilterData,
} from ".";

export function loadFilter(
  api: Api,
  filter: EntityFilter
): Promise<ValueMap[]> {
  if (filter.type === "sql") {
    if (filter.sql.trim()) {
      return api.selectSql(filter.sql);
    } else {
      return Promise.resolve([]);
    }
  } else {
    const select = buildFilterDataSelect(filter);
    return api.select(select);
  }
}

export function emptyEntityFilter(): EntityFilter {
  return {
    type: "data",
    searchTerm: "",
    entityTypes: [],
  };
}

export interface EntityFilterFormProps {
  initialFilter?: EntityFilter;

  onChange: (filter: EntityFilter) => void;
}

export function EntityFilterForm(props: EntityFilterFormProps): JSX.Element {
  const initialFilter = props.initialFilter ?? newFilterData();

  const [filter, setFilter] = createSignal<EntityFilter>(initialFilter);

  let oldDataFilter: EntityFilterData | undefined;
  let oldSqlFilter: EntityFilterSql | undefined;
  if (filterIsSql(initialFilter)) {
    oldSqlFilter = initialFilter;
  } else {
    oldDataFilter = initialFilter;
  }

  createEffect(() => {
    const current = filter();

    if (filterIsSql(current)) {
      oldSqlFilter = current;
    } else if (filterIsData(current)) {
      oldDataFilter = current;
    }
    props.onChange(current);
  });

  return (
    <div>
      <Tabs
        initialIndex={
          props.initialFilter ? (filterIsSql(props.initialFilter) ? 1 : 0) : 0
        }
        onChange={(index) => {
          if (index === 0) {
            setFilter(oldDataFilter ?? newFilterData());
          } else if (index === 1) {
            setFilter(oldSqlFilter ?? { type: "sql", sql: "" });
          } else {
            throw new Error("invalid tab index");
          }
        }}
        children={["Form", "SQL"]}
      />

      <Switch>
        <Match when={filter().type === "data"}>
          <FilterBuilder filter={filter as any} setFilter={setFilter} />
        </Match>

        <Match when={filter().type === "sql"}>
          <EntityFilterSqlForm filter={filter as any} setFilter={setFilter} />
        </Match>
      </Switch>

      <div></div>
    </div>
  );
}
