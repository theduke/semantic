import { useNavigate } from "solid-app-router";
import { ErrorBoundary, JSX } from "solid-js";
import { newUuid } from "..";
import { useApi, useRegistry } from "../../context";
import { routeEntityPage } from "../../routing";
import { EntityType } from "../../semantic";
import { ValueMap } from "../../semantic/registry";
import { NotificationError } from "../bulma/notification";
import { GenericPage } from "../util";
import { EntityCreator } from "./EntityCreator";

export interface EntityCreatePageProps {
  entityType: EntityType;
}

export function EntityCreatePage(props: EntityCreatePageProps): JSX.Element {
  const reg = useRegistry();
  const schema = reg.getEntityType(props.entityType);
  const api = useApi();
  const navigate = useNavigate();

  const typeTitle = schema?.["factor/title"] ?? schema?.["factor/ident"];

  const onSubmit = async (entity: ValueMap) => {
    const id = newUuid();
    await api.batch({
      actions: [
        {
          Create: {
            id,
            data: entity,
          },
        },
      ],
    });
    navigate(routeEntityPage(id));
  };

  const content = schema ? (
    <ErrorBoundary
      fallback={(e) => <NotificationError>{e.toString()}</NotificationError>}
    >
      <EntityCreator schema={schema} onSubmit={onSubmit} />
    </ErrorBoundary>
  ) : (
    <NotificationError>
      Entity type {props.entityType} not found
    </NotificationError>
  );
  return <GenericPage title={"New " + typeTitle}>{content}</GenericPage>;
}
