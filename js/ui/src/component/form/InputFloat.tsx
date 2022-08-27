import { JSX } from "solid-js";
import { FieldAccessor, MappedFieldAccessor } from ".";
import { StringInput } from "./StringInput";

export type InputType = "text" | "number" | "url" | "datetime-local";

export interface InputFloatProps {
  field: FieldAccessor<number | undefined>;
  placeholder?: string;
  mode?: "onchange" | "oninput";
}

export function InputFloat(props: InputFloatProps): JSX.Element {
  const mapped = new MappedFieldAccessor<number | undefined, string>(
    props.field,
    parseFloat,
    (v) => v?.toString() ?? ""
  );
  return (
    <StringInput
      type="number"
      field={mapped}
      placeholder={props.placeholder}
      mode={props.mode}
    />
  );
}
