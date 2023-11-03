import { ReactNode } from "react";
import { useParams } from 'react-router-dom';
import { useApi, useRegistry } from "../context";
import { LoaderView, useLoader } from "../loader";
import { ValueMap } from "@semantic/api";

export function RoutedEntityPage(): ReactNode {
  const { id } = useParams();
  return <EntityPage id={id!} />;
}

export interface EntityPageProps {
  id: string;
}

export function EntityPage(props: EntityPageProps): ReactNode {
  const api = useApi();
  const registry = useRegistry();

  const [state, _actions] = useLoader<ValueMap, string, any>(async (id: string) => {
    return api.entity(id);
  }, props.id);

  return <LoaderView state={state}
    render={entity => registry.renderEntity(entity, { preview: false, allowEdit: true, allowDelete: true })}
  />
}
