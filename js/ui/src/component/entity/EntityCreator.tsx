import { JSX } from "solid-js";
import { useApi, useRegistry } from "../../context";
import { Class } from "semantic/dist/core";
import { ValueMap } from "../../semantic/registry";
import { FACTOR_ENTITY_ATTRIBUTES, FACTOR_ID, SEMANTIC_CREATED_AT } from "semantic/dist/schema";
import { GenericEntityForm } from "./entity_form";
import { newUuid } from "..";
import { NotificationErrorBoundary } from "../util";

export interface EntityCreatorProps {
  schema: Class;

  cancelLabel?: JSX.Element;
  onCancel?: () => void;

  submitLabel?: JSX.Element;

  onSubmit?: (values: ValueMap) => Promise<void> | void;

  // Called once the entity has been created.
  // NOTE: only called when onSubmit is not specified.
  // If it is specified, then the user must manually persist.
  onPersisted?: (entity: ValueMap) => void;

  // TODO: enfore that either onSubmit or onPersisted is specified via union types
}

export function EntityCreator(props: EntityCreatorProps): JSX.Element {
  const reg = useRegistry();
  const submitLabel = props.submitLabel || "Create";

  const initialValues: ValueMap = {};

  const hasCreatedAt =
    props.schema[FACTOR_ENTITY_ATTRIBUTES].find(
      (x) => x['factor/attribute'] === SEMANTIC_CREATED_AT
    ) !== undefined;

  if (hasCreatedAt) {
    initialValues[SEMANTIC_CREATED_AT] = new Date().getTime();
  }

  let onSubmit = props.onSubmit;
  if (props.onPersisted) {
    const api = useApi();
    const onPersisted = props.onPersisted;
    onSubmit = async (values) => {
      const id = newUuid();
      await api.batch({ actions: [{ Create: { id, data: values } }] });
      onPersisted({ ...values, [FACTOR_ID]: id });
    };
  } else if (!props.onSubmit) {
    console.trace(
      "invalid EntityCreator props: must either specify onSubmit or onPersisted"
    );
    throw new Error(
      "invalid EntityCreator props: must either specify onSubmit or onPersisted"
    );
  }

  return (
    <NotificationErrorBoundary>
      <GenericEntityForm
        registry={reg}
        schema={props.schema}
        submitLabel={submitLabel}
        cancelLabel={props.cancelLabel}
        onCancel={props.onCancel}
        onSubmit={onSubmit}
        initialValues={initialValues}
      />
    </NotificationErrorBoundary>
  );
}
