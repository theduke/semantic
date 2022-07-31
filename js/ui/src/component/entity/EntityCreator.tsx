import { JSX } from "solid-js";
import { useRegistry } from "../../context";
import { EntitySchema } from "semantic/dist/core";
import { ValueMap } from "../../semantic/registry";
import { SEMANTIC_CREATED_AT } from "semantic/dist/schema";
import { GenericEntityForm } from "./entity_form";

export interface EntityCreatorProps {
  schema: EntitySchema;

  cancelLabel?: JSX.Element;
  onCancel?: () => void;

  submitLabel?: JSX.Element;
  onSubmit?: (values: ValueMap) => Promise<void> | void;
}

export function EntityCreator(props: EntityCreatorProps): JSX.Element {
  const reg = useRegistry();
  const submitLabel = props.submitLabel || "Create";

  const initialValues: ValueMap = {};

  const hasCreatedAt =
    props.schema["factor/entityAttributes"].find(
      (x) => x.attribute === SEMANTIC_CREATED_AT
    ) !== undefined;

  if (hasCreatedAt) {
    initialValues[SEMANTIC_CREATED_AT] = new Date().getTime();
  }

  return (
    <GenericEntityForm
      registry={reg}
      schema={props.schema}
      submitLabel={submitLabel}
      cancelLabel={props.cancelLabel}
      onCancel={props.onCancel}
      onSubmit={props.onSubmit}
      initialValues={initialValues}
    />
  );
}
