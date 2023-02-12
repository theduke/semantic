import { JSX } from "solid-js/jsx-runtime";
import { useApi } from "../../context";
import { IdOrIdent } from "semantic/dist/core";
import { ValueMap } from "../../semantic/registry";
import { createResource } from "solid-js";
import { ResourceViewer } from "../util/load";

export interface EntityLoaderProps {
  ident: IdOrIdent;
  children: (item: ValueMap) => JSX.Element;
}

export function EntityLoader(props: EntityLoaderProps): JSX.Element {
  const api = useApi();

  const [res] = createResource(() => api.entity(props.ident));

  return <ResourceViewer resource={res} children={props.children} />;
}
