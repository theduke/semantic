import { JSX } from "solid-js/jsx-runtime";
import { useApi } from "../../context";
import { IdOrIdent } from "semantic/dist/core";
import { ValueMap } from "../../semantic/registry";
import { BoundarySuspenseLoader } from "../util/load";

export interface EntityLoaderProps {
  ident: IdOrIdent;
  children: (item: ValueMap) => JSX.Element;
}

export function EntityLoader(props: EntityLoaderProps): JSX.Element {
  const api = useApi();

  return (
    <BoundarySuspenseLoader<ValueMap>
      load={() => api.entity(props.ident)}
      render={props.children}
    />
  );
}
