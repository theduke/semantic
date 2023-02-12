import { JSX } from "solid-js/jsx-runtime";
import { useApi } from "../../context";
import { Select } from "semantic/dist/core";
import { ValueMap } from "../../semantic/registry";
import { ResourceViewer } from "../util/load";
import { createResource } from "solid-js";

export interface EntititesLoaderProps {
  select: Select;
  children: (items: ValueMap[]) => JSX.Element;
  emptyFallback?: JSX.Element;
}

export function EntitiesLoader(props: EntititesLoaderProps): JSX.Element {
  const api = useApi();

  const [res] = createResource(() => api.select(props.select));

  const renderer = props.emptyFallback
    ? (values: ValueMap[]) => {
        if (values.length === 0 && props.emptyFallback) {
          return props.emptyFallback;
        } else {
          return props.children(values);
        }
      }
    : props.children;

  return <ResourceViewer resource={res} children={renderer} />;
}
