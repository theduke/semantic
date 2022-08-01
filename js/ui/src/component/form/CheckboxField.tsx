import { JSX, splitProps } from "solid-js";
import { FormField, FormFieldProps } from "./FormField";
import { Checkbox } from "./Checkbox";

export interface CheckboxFieldProps
  extends Omit<FormFieldProps<boolean>, "control"> {
  checkboxLabel: string;
}

export function CheckboxField(props: CheckboxFieldProps): JSX.Element {
  return (
    <FormField
      {...props}
      control={<Checkbox label={props.checkboxLabel} field={props.field} />}
    />
  );
}
