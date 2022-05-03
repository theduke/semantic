import { For, JSX, Show } from "solid-js";
import { FieldAccessor } from ".";

export interface FormFieldProps<T> {
  label: JSX.Element;
  help?: JSX.Element;
  field: FieldAccessor<T>;
  control: JSX.Element;
}

export function FormField<T>(props: FormFieldProps<T>): JSX.Element {
  return (
    <div class="field">
      <div className="label">{props.label}</div>
      <div className="control">{props.control}</div>
      {props.help ? <p class="help">{props.help}</p> : null}
      {renderFieldErrors(props.field)}
    </div>
  );
}

function renderFieldErrors(field: FieldAccessor<any>): JSX.Element {
  return (
    <>
      <Show when={field.errors()?.errors?.length ?? 0 > 0}>
        <p className="help is-danger">
          <For each={field.errors()?.errors ?? []}>
            {(error) => <div className="mb-2">{error.message}</div>}
          </For>
        </p>
      </Show>

      <Show when={field.errors()?.warnings?.length ?? 0 > 0}>
        <p className="help is-warning">
          <For each={field.errors()?.warnings ?? []}>
            {(error) => <div className="mb-2">{error.message}</div>}
          </For>
        </p>
      </Show>
    </>
  );
}
