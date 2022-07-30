import { JSX } from "solid-js";
import { DateTimeInput } from "./DateTimeInput";
import { FormField, FormFieldProps } from "./FormField";

export interface DateTimeInputFieldProps
  extends Omit<FormFieldProps<number>, "control"> {}

export function DateTimeInputField(
  props: DateTimeInputFieldProps
): JSX.Element {
  return (
    <FormField {...props} control={<DateTimeInput field={props.field} />} />
  );
}
