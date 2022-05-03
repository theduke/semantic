import { createSignal, Show } from "solid-js";
import { JSX } from "solid-js/jsx-runtime";
import { useApi } from "../../context";
import { genericTypedEntityTitle } from "../../semantic";
import { BaseEntity } from "../../semantic/schema";
import { Button, Buttons } from "../bulma/button";
import { NotificationError } from "../bulma/notification";
import { LoadState } from "../util/load";

export interface EntityDeleterProps<E extends BaseEntity> {
  entity: E;
  customName?: string;
  confirmationContent?: JSX.Element;
  onDeleted?: (entity: E) => void;
  onCancel?: () => void;
}

export function EntityDeleter<E extends BaseEntity>(
  props: EntityDeleterProps<E>
): JSX.Element {
  const api = useApi();
  const [state, setState] = createSignal<LoadState<void>>({ state: "idle" });

  let content = props.confirmationContent;
  if (!content) {
    const name = props.customName ?? genericTypedEntityTitle(props.entity);
    content = <p>Really delete {name}?</p>;
  }

  const runDelete = () => {
    setState({ state: "loading" });

    api
      .mutate({ Delete: { id: props.entity["factor/id"] } })
      .then(() => {
        setState({ state: "success", data: undefined });
        props?.onDeleted?.(props.entity);
      })
      .catch((error) => {
        setState({ state: "error", error: error });
      });
  };

  return (
    <div class="box">
      <div class="mb-5">{content}</div>

      <Show when={state().state === "error"}>
        {() => {
          const s = state();
          const error =
            s.state === "error" ? s.error.toString() : "Unknown error";
          return (
            <div class="mb-3 mt-3">
              <NotificationError>{error}</NotificationError>
            </div>
          );
        }}
      </Show>

      <Buttons>
        <Button
          color="is-danger"
          loading={state().state === "loading"}
          onclick={runDelete}
        >
          Delete
        </Button>
        {props.onCancel ? (
          <Button onclick={props.onCancel}>Cancel</Button>
        ) : null}
      </Buttons>
    </div>
  );
}
