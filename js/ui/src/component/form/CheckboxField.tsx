import { JSX } from "solid-js";
import { FormField, FormFieldProps } from "./FormField";
import { Checkbox } from "./Checkbox";

export interface CheckboxFieldProps
  extends Omit<FormFieldProps<boolean>, "control"> {}

export function CheckboxField(props: CheckboxFieldProps): JSX.Element {
  return <FormField {...props} control={<Checkbox field={props.field} />} />;
}
