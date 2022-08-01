import { JSX } from "solid-js";
import { Icon, IconName } from "../bulma/icon";
import { FormField, FormFieldProps } from "./FormField";
import { Input, InputType } from "./Input";

export interface InputFieldProps
  extends Omit<FormFieldProps<string>, "control"> {
  placeholder?: string;
  type?: InputType;
  icon?: IconName;
  mode?: "onchange" | "oninput";
}

export function InputField(props: InputFieldProps): JSX.Element {
  return (
    <FormField
      {...props}
      controlExtraClasses={props.icon ? "has-icons-left" : ""}
      control={[
        <Input
          type={props.type}
          placeholder={props.placeholder}
          field={props.field}
          mode={props.mode}
        />,
        props.icon ? <Icon icon={props.icon} isLeft /> : undefined,
      ]}
    />
  );
}
