import { createEffect, JSX } from "solid-js";
import { FieldAccessor, MappedFieldAccessor } from ".";
import { StringInput } from "./StringInput";

export type InputType = "text" | "number" | "url" | "datetime-local";

export interface InputFloatProps {
  field: FieldAccessor<number | undefined>;
  placeholder?: string;
  mode?: "onchange" | "oninput";
  min?: number;
  max?: number;
}

function parseFloatRange(value: string, min: number|undefined, max: number|undefined): number {
  const num = parseFloat(value);
  if (min !== undefined && num < min) {
    throw new Error('Number must be >= ' + min);
  } else if (max !== undefined && num > max) {
    throw new Error('Number must be <= ' + max);
  }
  return num;
}

export function InputFloat(props: InputFloatProps): JSX.Element {
  const mapped = new MappedFieldAccessor<number | undefined, string>(
    props.field,
    raw => parseFloatRange(raw, props.min, props.max),
    (v) => v?.toString() ?? ""
  );
  return (
    <StringInput
      type="number"
      min={props.min}
      max={props.max}
      field={mapped}
      placeholder={props.placeholder}
      mode={props.mode}
    />
  );
}
