import { Box as div } from "solid-bulma";
import { createEffect, createSignal, JSX, Match, Switch } from "solid-js";
import { Select } from "../../../semantic/core";
import { Tabber, Tabs } from "../../bulma/tabs";
import {
  EntityFilterData,
  EntityFilterForm,
  newFilterData,
} from "./EntityFilterForm";
import { EntityFilterSql, EntityFilterSqlForm } from "./EntityFilterSqlForm";

export type EntityFilter = EntityFilterData | EntityFilterSql;

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

export function EntityFilter(props: EntityFilterProps): JSX.Element {
  const [filter, setFilter] = createSignal<EntityFilter>(
    props.initialFilter ?? newFilterData()
  );

  createEffect(() => {
    props.onChange(filter());
  });

  return (
    <div>
      <Tabs
        onChange={(index) => {
          if (index === 0) {
            setFilter(newFilterData());
          } else if (index === 1) {
            setFilter({ type: "sql", sql: "" });
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
