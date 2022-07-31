import { JSX } from "solid-js";
import { useApi, useRegistry } from "../../context";
import { Mutate } from "semantic/dist/core";
import { ValueMap } from "../../semantic/registry";
import { FACTOR_ID, FACTOR_TYPE } from "semantic/dist/schema";
import { GenericEntityForm } from "./entity_form";

export type PersistanceMode = "merge" | "replace";

export interface EntityEditorProps {
  item: ValueMap;

  onSaved: (item: ValueMap) => void;
  submitLabel?: JSX.Element;

  mode?: PersistanceMode;

  onCancel?: () => void;
  cancelLabel?: JSX.Element;
}

export function EntityEditor(props: EntityEditorProps): JSX.Element {
  const reg = useRegistry();
  const submitLabel = props.submitLabel || "Save";
  const api = useApi();

  const ty = props.item[FACTOR_TYPE];
  const schema = ty ? reg.getEntityType(ty) : undefined;

  const mode = props.mode || "replace";
  const onSubmit = async (values: ValueMap) => {
    let action: Mutate;

    switch (mode) {
      case "merge":
        action = { Merge: { id: props.item[FACTOR_ID], data: values } };
        break;
      case "replace":
        action = { Replace: { id: props.item[FACTOR_ID], data: values } };
    }
    await api.mutate(action);
    props.onSaved(values);
  };

  console.debug("rendering entity editor");

  return (
    <GenericEntityForm
      registry={reg}
      schema={schema}
      submitLabel={submitLabel}
      cancelLabel={props.cancelLabel}
      onCancel={() => {
        props.onCancel?.();
      }}
      onSubmit={onSubmit}
      initialValues={props.item}
    />
  );
}
