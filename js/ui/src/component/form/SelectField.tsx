import { JSX } from "solid-js";
import { FormField, FormFieldProps } from "./FormField";
import { SelectOption, Select } from "./Select";

export interface SelectFieldProps<T>
  extends Omit<FormFieldProps<T>, "control"> {
  options: SelectOption<T>[];
}

export function SelectField<T>(props: SelectFieldProps<T>): JSX.Element {
  return (
    <FormField
      {...props}
      control={<Select<T> options={props.options} field={props.field} />}
    />
  );
}
