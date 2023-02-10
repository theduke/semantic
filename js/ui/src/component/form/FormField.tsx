import { createEffect, For, JSX, Show } from "solid-js";
import { FieldAccessor } from ".";

export interface FormFieldProps<T> {
  label: JSX.Element;
  help?: JSX.Element;
  field: FieldAccessor<T>;
  control: JSX.Element;
  controlExtraClasses?: string;
}

export function FormField<T>(props: FormFieldProps<T>): JSX.Element {
  return (
    <div class="field">
      {props.label ? <div class="label">{props.label}</div> : null}
      <div
        class={
          "control" +
          (props.controlExtraClasses ? " " + props.controlExtraClasses : "")
        }
      >
        {props.control}
      </div>
      {props.help ? <p class="help">{props.help}</p> : null}
      <FieldErrors field={props.field} />
    </div>
  );
}

function FieldErrors(props: { field: FieldAccessor<any> }): JSX.Element {
  const field = props.field;
  createEffect(() => {
    const errors = field.errors()?.errors;
    console.debug({ field, fieldErrrs: errors, form: (field as any).form });
  });
  return (
    <>
      <Show when={field.errors()?.errors?.length ?? 0 > 0}>
        <p class="help is-danger">
          <For each={field.errors()?.errors ?? []}>
            {(error) => <div class="mb-2">{error.message}</div>}
          </For>
        </p>
      </Show>

      <Show when={field.errors()?.warnings?.length ?? 0 > 0}>
        <p class="help is-warning">
          <For each={field.errors()?.warnings ?? []}>
            {(error) => <div class="mb-2">{error.message}</div>}
          </For>
        </p>
      </Show>
    </>
  );
}
