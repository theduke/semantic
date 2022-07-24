import { useNavigate } from "solid-app-router";
import { ErrorBoundary, JSX } from "solid-js";
import { newUuid } from "..";
import { useApi, useRegistry } from "../../context";
import { routeEntityPage } from "../../routing";
import { EntityType } from "../../semantic";
import { ValueMap } from "../../semantic/registry";
import { NotificationError } from "../bulma/notification";
import { GenericPage } from "../util";
import { GenericEntityForm } from "./entity_form";

export interface EntityCreatePageProps {
  entityType: EntityType;
}

export function EntityCreatePage(props: EntityCreatePageProps): JSX.Element {
  const reg = useRegistry();
  const schema = reg.getEntityType(props.entityType);
  const api = useApi();
  const navigate = useNavigate();

  const onSubmit = async (entity: ValueMap) => {
    const id = newUuid();
    await api.batch({
      actions: [{
        'Create': {
          id,
          data: entity,
        },
      }],
    });
    navigate(routeEntityPage(id));
  };

  const content = schema ? (
    <ErrorBoundary
      fallback={(e) => <NotificationError>{e.toString()}</NotificationError>}
    >
      <GenericEntityForm
        registry={reg}
        schema={schema}
        submitLabel={"Create"}
        onSubmit={onSubmit}
      />
    </ErrorBoundary>
  ) : (
    <NotificationError>
      Entity type {props.entityType} not found
    </NotificationError>
  );
  return (
    <GenericPage title={"Create " + props.entityType}>{content}</GenericPage>
  );
}
