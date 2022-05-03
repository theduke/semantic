import { JSX } from "solid-js/jsx-runtime";
import { useApi } from "../../context";
import { IdOrIdent, Item } from "../../semantic/core";
import { BoundarySuspenseLoader } from "../util/load";

export interface EntityLoaderProps {
  ident: IdOrIdent;
  children: (item: Item) => JSX.Element;
}

export function EntityLoader(props: EntityLoaderProps): JSX.Element {
  const api = useApi();

  return (
    <BoundarySuspenseLoader<Item>
      load={() => api.entity(props.ident)}
      render={props.children}
    />
  );
}
