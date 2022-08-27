import { JSX } from "solid-js";
import { Icon, IconName } from "../bulma/icon";
import { FormField, FormFieldProps } from "./FormField";
import { InputInt } from "./InputInt";

export interface InputIntFieldProps
  extends Omit<FormFieldProps<number>, "control"> {
  placeholder?: string;
  icon?: IconName;
  mode?: "onchange" | "oninput";
}

export function InputIntField(props: InputIntFieldProps): JSX.Element {
  return (
    <FormField
      {...props}
      controlExtraClasses={props.icon ? "has-icons-left" : ""}
      control={[
        <InputInt
          placeholder={props.placeholder}
          field={props.field}
          mode={props.mode}
        />,
        props.icon ? <Icon icon={props.icon} isLeft /> : undefined,
      ]}
    />
  );
}
