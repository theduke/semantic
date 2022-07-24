import { Accessor, JSX } from "solid-js";
import { isAccessor } from "../util";

export interface ModalProps {
  children: JSX.Element;
  isActive?: Accessor<boolean> | boolean | null;
  onClose?: () => void;
  overflow?: "auto" | "hidden" | "scroll" | "visible";
}

export function Modal(props: ModalProps): JSX.Element {
  const isActive = props.isActive;
  const accessor = isAccessor(isActive);

  const clsList = {
    modal: true,
    "is-active": accessor ? isActive() : true,
  };

  const onClose = props.onClose
    ? (e: MouseEvent) => {
        e.stopPropagation();
        props.onClose?.();
      }
    : undefined;

  // TODO: better solution for background color
  return (
    <div classList={clsList}>
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
