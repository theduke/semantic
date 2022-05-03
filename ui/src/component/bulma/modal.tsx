import { Accessor, JSX } from "solid-js";

export interface ModalProps {
  children: JSX.Element;
  isActive?: Accessor<boolean> | boolean | null;
  onClose?: () => void;
  overflow?: "auto" | "hidden" | "scroll" | "visible";
}

function isAccessor<T>(
  value: Accessor<T> | T | null | undefined
): value is Accessor<T> {
  return typeof value === "function";
}

export function Modal(props: ModalProps): JSX.Element {
  const tyActive = typeof props.isActive;

  const cls =
    "modal" +
    (props.isActive
      ? tyActive === "boolean"
        ? " is-active"
        : ""
      : " is-active");
  const clsList = isAccessor(props.isActive)
    ? { "is-active": props.isActive() }
    : undefined;

  const onClose = props.onClose
    ? (e: MouseEvent) => {
        e.stopPropagation();
        props.onClose?.();
      }
    : undefined;

  // TODO: better solution for background color
  return (
    <div class={cls} classList={clsList}>
      <div
        class="modal-background"
        style={{ "background-color": "rgba(10,10,10,.20)" }}
        onclick={onClose}
      ></div>
      <div class="modal-content" style={{ overflow: props.overflow }}>
        {props.children}
      </div>
      <button
        class="modal-close is-large"
        aria-label="close"
        onclick={onClose}
      />
    </div>
  );
}
