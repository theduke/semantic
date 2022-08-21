import { createEffect, JSX, onMount } from "solid-js";
import { FieldAccessor } from ".";

export interface CheckboxProps {
  label: string;
  field: FieldAccessor<boolean>;
}

export function Checkbox(props: CheckboxProps): JSX.Element {
  const field = props.field;
  let elem: HTMLInputElement | undefined;

  onMount(() => {
    if (elem) {
      console.log({ checkboxValue: props.field.get().value });
      elem.checked = props.field.get()?.value ?? false;
    }
    createEffect(() => {
      if (elem) {
        elem.checked = props.field.get()?.value ?? false;
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
    <label class="checkbox">
      <input
        ref={elem}
        type="checkbox"
        onchange={(e) => {
          e.preventDefault();
          e.stopPropagation();
          field.set(e.currentTarget.checked);
        }}
      />{" "}
      {props.label}
    </label>
  );
}
