import { JSX } from "solid-js";
import { FormField, FormFieldProps } from "./FormField";
import { Input, InputType } from "./Input";

export interface InputFieldProps
  extends Omit<FormFieldProps<string>, "control"> {
  placeholder?: string;
  type?: InputType;
}

export function InputField(props: InputFieldProps): JSX.Element {
  return (
    <FormField
      {...props}
      control={
        <Input
          type={props.type}
          placeholder={props.placeholder}
          field={props.field}
        />
      }
    />
  );
}
