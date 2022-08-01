import { JSX } from "solid-js/jsx-runtime";
import { useApi } from "../../context";
import { Select } from "semantic/dist/core";
import { ValueMap } from "../../semantic/registry";
import { BoundarySuspenseLoader } from "../util/load";

export interface EntititesLoaderProps {
  select: Select;
  children: (items: ValueMap[]) => JSX.Element;
  emptyFallback?: JSX.Element;
}

export function EntitiesLoader(props: EntititesLoaderProps): JSX.Element {
  const api = useApi();

  const renderer = props.emptyFallback
    ? (values: ValueMap[]) => {
        if (values.length === 0) {
          return props.emptyFallback;
        } else {
          return props.children(values);
        }
      }
    : props.children;

  return (
    <BoundarySuspenseLoader<ValueMap[]>
      load={() => api.select(props.select)}
      render={renderer}
    />
  );
}
