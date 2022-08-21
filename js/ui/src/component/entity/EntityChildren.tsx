import { For, JSX } from "solid-js";
import { newSelect } from "semantic/dist/api";
import { useRegistry } from "../../context";
import { Expr, Id, Select, Sort } from "semantic/dist/core";
import { exprAnd } from "semantic/dist/db";
import { EntitiesLoader } from "./EntitiesLoader";

export interface EntityChildrenProps {
  id: Id;
  baseFilter?: Expr;
  sort?: Sort;
}

export function EntityChildrenLoader(props: EntityChildrenProps): JSX.Element {
  let filter: Expr = {
    BinaryOp: {
      op: "Eq",
      left: { Attr: "semantic/parent" },
      right: { Literal: props.id },
    },
  };
  filter = props.baseFilter ? exprAnd(filter, props.baseFilter) : filter;
  const select: Select = {
    ...newSelect(),
    filter,
    limit: 500 as any,
  };

  const reg = useRegistry();

  return (
    <EntitiesLoader select={select}>
      {(items) => (
        <For each={items}>
          {(item) => reg.renderEntity(item, { preview: true })}
        </For>
      )}
    </EntitiesLoader>
  );
}
