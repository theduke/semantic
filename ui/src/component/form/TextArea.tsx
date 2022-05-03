import { createEffect, JSX, onMount } from "solid-js";
import { FieldAccessor } from ".";

export interface TextAreaProps {
  field: FieldAccessor<string>;
  placeholder?: string;
}

export function TextArea(props: TextAreaProps): JSX.Element {
  const field = props.field;
  let elem: HTMLTextAreaElement | undefined;

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
    <textarea
      placeholder={props.placeholder}
      class="textarea"
      ref={elem}
      onchange={(e) => {
        e.preventDefault();
        e.stopPropagation();

        const value = elem?.value;
        field.set(value ?? "");
      }}
    />
  );
}
