import { createEffect, JSX, onMount } from "solid-js";
import { FieldAccessor } from ".";

export type InputType = "text" | "number" | "url" | "datetime-local";

export interface DateTimeInputProps {
  field: FieldAccessor<number>;
}

export function DateTimeInput(props: DateTimeInputProps): JSX.Element {
  const field = props.field;
  let elem: HTMLInputElement | undefined;

  onMount(() => {
    createEffect(() => {
      const field = props.field.get();
      if (elem) {
        if (field?.value) {
          const date = new Date(field.value);
          if (!isNaN(date as any)) {
            elem.value = date.toISOString();
          }
        }
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
      placeholder={"YYYY-MM-DDTHH:MM:SS.SSSZ"}
      class="input"
      ref={elem}
      type="text"
      onchange={(e) => {
        e.preventDefault();
        e.stopPropagation();
        const value = elem?.value?.trim();
        if (value) {
          const date = new Date(value);
          if (!isNaN(date as any)) {
            field.set(date.getTime());
          }
        }
      }}
    />
  );
}
