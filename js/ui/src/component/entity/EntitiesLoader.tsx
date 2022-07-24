import { JSX } from "solid-js/jsx-runtime";
import { useApi } from "../../context";
import { Select } from "../../semantic/core";
import { ValueMap } from "../../semantic/registry";
import { BoundarySuspenseLoader } from "../util/load";

export interface EntititesLoaderProps {
  select: Select;
  children: (items: ValueMap) => JSX.Element;
}

export function EntitiesLoader(props: EntititesLoaderProps): JSX.Element {
  const api = useApi();

  return (
    <BoundarySuspenseLoader<ValueMap>
      load={() => api.select(props.select)}
      render={props.children}
    />
  );
}
