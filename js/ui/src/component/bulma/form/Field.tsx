import { JSX } from "solid-js";

export interface FieldProps {
  label: JSX.Element;
  help?: JSX.Element;
  children: JSX.Element;
  errors?: JSX.Element[];
}

export function FormField(props: FieldProps): JSX.Element {
  return (
    <div class="field">
      <div class="label">{props.label}</div>class
      <div class="control">{props.children}</div>
      {props.help ? <p class="help">{props.help}</p> : null}
      {props.errors}
    </div>
  );
}
