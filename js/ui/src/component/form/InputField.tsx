import { JSX } from "solid-js";
import { FormField, FormFieldProps } from "./FormField";
import { Input } from "./Input";

export interface InputFieldProps
  extends Omit<FormFieldProps<string>, "control"> {
  placeholder?: string;
}

export function InputField(props: InputFieldProps): JSX.Element {
  return (
    <FormField
      {...props}
      control={<Input placeholder={props.placeholder} field={props.field} />}
    />
  );
}
