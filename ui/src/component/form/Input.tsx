import { createEffect, JSX, onMount } from "solid-js";
import { FieldAccessor } from ".";

export interface InputProps {
  field: FieldAccessor<string>;
  placeholder?: string;
}

export function Input(props: InputProps): JSX.Element {
  const field = props.field;
  let elem: HTMLInputElement | undefined;

  onMount(() => {
    createEffect(() => {
      const field = props.field.get();
      if (elem) {
        elem.value = field?.value ?? "";
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

  return (
    <input
      placeholder={props.placeholder}
      class="input"
      ref={elem}
      type="text"
      onchange={(e) => {
        e.preventDefault();
        e.stopPropagation();

        const value = elem?.value;
        field.set(value ?? "");
      }}
    />
  );
}
