import { JSX } from "solid-js";
import { FieldAccessor, MappedFieldAccessor } from ".";
import { StringInput } from "./StringInput";

export interface InputIntProps {
  field: FieldAccessor<number | undefined>;
  placeholder?: string;
  mode?: "onchange" | "oninput";
}

export function InputInt(props: InputIntProps): JSX.Element {
  const mapped = new MappedFieldAccessor<number | undefined, string>(
    props.field,
    parseInt,
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
