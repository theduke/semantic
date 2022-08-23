import { createEffect, createSignal, JSX, Match, Switch } from "solid-js";
import { Tabs } from "../../bulma/tabs";
import {
  buildFilterDataSelect,
  EntityFilterData,
  EntityFilterForm,
  newFilterData,
  validateEntityFilterData,
} from "./EntityFilterForm";
import {
  EntityFilterSql,
  EntityFilterSqlForm,
  validateEntityFilterSql,
} from "./EntityFilterSqlForm";
import zod from "zod";
import { Api, ValueMap } from "semantic/dist/api";

export const validateEntityFilter = zod.discriminatedUnion("type", [
  validateEntityFilterData,
  validateEntityFilterSql,
]);
export type EntityFilter = zod.infer<typeof validateEntityFilter>;

export interface EntityFilterProps {
  initialFilter?: EntityFilter;

  onChange: (filter: EntityFilter) => void;
}

function isSql(filter: EntityFilter): filter is EntityFilterSql {
  return filter.type === "sql";
}

function isData(filter: EntityFilter): filter is EntityFilterData {
  return filter.type === "data";
}

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

export function EntityFilter(props: EntityFilterProps): JSX.Element {
  const initialFilter = props.initialFilter ?? newFilterData();

  const [filter, setFilter] = createSignal<EntityFilter>(initialFilter);

  let oldDataFilter: EntityFilterData | undefined;
  let oldSqlFilter: EntityFilterSql | undefined;
  if (isSql(initialFilter)) {
    oldSqlFilter = initialFilter;
  } else {
    oldDataFilter = initialFilter;
  }

  createEffect(() => {
    const current = filter();

    if (isSql(current)) {
      oldSqlFilter = current;
    } else if (isData(current)) {
      oldDataFilter = current;
    }
    props.onChange(current);
  });

  return (
    <div>
      <Tabs
        initialIndex={
          props.initialFilter ? (isSql(props.initialFilter) ? 1 : 0) : 0
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
          <EntityFilterForm filter={filter as any} setFilter={setFilter} />
        </Match>

        <Match when={filter().type === "sql"}>
          <EntityFilterSqlForm filter={filter as any} setFilter={setFilter} />
        </Match>
      </Switch>
    </div>
  );
}
