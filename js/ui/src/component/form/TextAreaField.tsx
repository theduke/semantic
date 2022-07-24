import { JSX } from "solid-js";
import { FormField, FormFieldProps } from "./FormField";
import { TextArea } from "./TextArea";

export interface TextAreaFieldProps
  extends Omit<FormFieldProps<string>, "control"> {
  placeholder?: string;
}

export function TextAreaField(props: TextAreaFieldProps): JSX.Element {
  return (
    <FormField
      {...props}
      control={<TextArea placeholder={props.placeholder} field={props.field} />}
    />
  );
}
