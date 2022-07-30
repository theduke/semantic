import { For, JSX } from "solid-js";
import { newSelect } from "../../api";
import { useRegistry } from "../../context";
import { Expr, Id, Select, Sort } from "../../semantic/core";
import { exprAnd } from "../../semantic/db";
import { EntitiesLoader } from "./EntitiesLoader";

export interface EntityChildrenProps {
  id: Id;
  baseFilter?: Expr;
  sort?: Sort;
}

export function EntityChildren(props: EntityChildrenProps): JSX.Element {
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
