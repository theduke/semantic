import { createEffect, JSX, onMount } from "solid-js";
import { FieldAccessor } from ".";

export type InputType = "text" | "number" | "url" | "datetime-local";

export type StringInputPropsNumber = {
  type: "number";
  min?: number;
  max?: number;
};

export type StringInputPropsUntyped = {
  type: InputType;
};

export interface StringInputBaseProps {
  field: FieldAccessor<string>;
  placeholder?: string;
  mode?: "onchange" | "oninput";
}

export type StringInputProps = StringInputBaseProps &
  (StringInputPropsNumber | StringInputPropsUntyped);

export function StringInput(props: StringInputProps): JSX.Element {
  const field = props.field;
  let elem: HTMLInputElement | undefined;

  onMount(() => {
    createEffect(() => {
      const field = props.field.get();
      const value = field?.value;
      if (elem && value !== undefined) {
        elem.value = value;
      }
    });

    createEffect(() => {
      if (!elem) {
        return;
      }
      const val = field.errors();
      if (val?.errors?.length ?? 0 > 0) {
        elem.classList.add("is-danger");
      } else if (val?.warnings?.length ?? 0 > 0) {
        elem.classList.remove("is-danger");
        elem.classList.add("is-warning");
      } else {
        elem.classList.remove("is-danger");
        elem.classList.remove("is-warning");
      }
    });
  });

  // TODO: avoid cast...
  const numProps: StringInputPropsNumber | null =
    props.type === "number" ? (props as StringInputPropsNumber) : null;

  return (
    <input
      placeholder={props.placeholder}
      class="input"
      ref={elem}
      type={props.type ?? "text"}
      min={numProps?.min}
      max={numProps?.max}
      onchange={
        props.mode === "onchange"
          ? (e) => {
              e.stopPropagation();
              const value = elem?.value;
              field.set(value ?? "");
            }
          : undefined
      }
      oninput={
        props.mode === "oninput"
          ? (e) => {
              e.stopPropagation();
              const value = elem?.value;
              field.set(value ?? "");
            }
          : undefined
      }
    />
  );
}
