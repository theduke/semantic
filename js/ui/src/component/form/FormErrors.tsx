import { For, JSX, Show } from "solid-js";
import { FormState } from ".";

export interface FormErrorsProps {
  form: FormState<any>;
}

export function FormErrors(props: FormErrorsProps): JSX.Element {
  const form = props.form;
  return (
    <>
      <Show when={form.state.validation?.form?.errors?.length ?? 0 > 0}>
        <p class="notification is-danger">
          <For each={form.state.validation?.form?.errors ?? []}>
            {(error) => <div class="mb-2">{error.message}</div>}
          </For>
        </p>
      </Show>

      <Show when={form.state.validation?.form?.warnings?.length ?? 0 > 0}>
        <p class="notification is-warning">
          <For each={form.state.validation?.form?.warnings ?? []}>
            {(warning) => <div class="mb-2">{warning.message}</div>}
          </For>
        </p>
      </Show>
    </>
  );
}
