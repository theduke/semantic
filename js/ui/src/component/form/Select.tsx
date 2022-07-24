import { createEffect, JSX, onMount } from "solid-js";
import { FieldAccessor } from ".";

export interface SelectOption<T> {
  label: JSX.Element;
  value: T;
}

interface IndexedOption<T> extends SelectOption<T> {
  index: number;
}

export interface SelectProps<T> {
  field: FieldAccessor<T>;
  options: SelectOption<T>[];
}

export function Select<T>(props: SelectProps<T>): JSX.Element {
  const field = props.field;
  let elem: HTMLSelectElement | undefined;

  // TODO: remove that any cast...
  const indexedOptions: IndexedOption<T>[] = props.options.map(
    (opt, index) => ({ ...opt, index })
  );

  onMount(() => {
    createEffect(() => {
      const field = props.field.get();
      const index = indexedOptions.findIndex(
        (opt) => opt.value === field?.value
      );

      if (elem && index >= 0) {
        elem.value = index.toString();
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
    <select
      class="input"
      ref={elem}
      onchange={(e) => {
        e.preventDefault();
        e.stopPropagation();
        const value = elem?.value;
        if (value) {
          field.set(indexedOptions[parseInt(value ?? "")].value);
        } else {
          field.set(undefined as any);
        }
      }}
    >
      {indexedOptions.map((opt) => (
        <option value={opt.index.toString()}>{opt.label}</option>
      ))}
    </select>
  );
}
