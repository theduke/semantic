import { JSX } from "solid-js";
import { Icon, IconName } from "../bulma/icon";
import { FormField, FormFieldProps } from "./FormField";
import { InputFloat } from "./InputFloat";

export interface InputFloatFieldProps
  extends Omit<FormFieldProps<number>, "control"> {
  placeholder?: string;
  icon?: IconName;
  mode?: "onchange" | "oninput";
}

export function InputFloatField(props: InputFloatFieldProps): JSX.Element {
  return (
    <FormField
      {...props}
      controlExtraClasses={props.icon ? "has-icons-left" : ""}
      control={[
        <InputFloat
          placeholder={props.placeholder}
          field={props.field}
          mode={props.mode}
        />,
        props.icon ? <Icon icon={props.icon} isLeft /> : undefined,
      ]}
    />
  );
}
