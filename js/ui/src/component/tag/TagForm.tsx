import { JSX } from "solid-js";
import { createForm } from "../form";
import { FormFooter } from "../form/FormFooter";
import { InputField } from "../form/InputField";

export interface TagFormValues {
  name: string;
}

export interface TagFormProps {
  submitLabel: JSX.Element;
  onSubmit: (values: TagFormValues) => Promise<void>;
  onCancel?: () => void;
  resetOnSubmit?: boolean;
}

export function TagForm(props: TagFormProps): JSX.Element {
  const form = createForm<TagFormValues>({
    initialValues: { name: "" },
    validate: (values) => {
      if (values.name.trim().length < 1) {
        return {
          fields: { name: { errors: [{ message: "Tag name is required" }] } },
        };
      } else {
        return null;
      }
    },

    onSubmit: async (values, form) => {
      values.name = values.name.trim();
      await props.onSubmit(values);
      if (props.resetOnSubmit) {
        form.reset();
      }
    },
  });

  return (
    <form
      onsubmit={(e) => {
        form.handleSubmit(e);
      }}
    >
      <InputField mode="onchange" field={form.field("name")} label="Name" />

      <FormFooter
        form={form}
        buttons={{
          submit: { children: props.submitLabel, color: "is-info" },
          onCancel: props.onCancel,
        }}
      />
    </form>
  );
}
