import { JSX } from "solid-js";
import { BaseEntity } from "../../semantic/schema";
import { Modal } from "../bulma/modal";
import { EntityDeleter, EntityDeleterProps } from "./EntityDeleter";

export interface EntityDeleterModalProps<E extends BaseEntity>
  extends EntityDeleterProps<E> {}

export function EntityDeleterModal<E extends BaseEntity>(
  props: EntityDeleterModalProps<E>
): JSX.Element {
  return (
    <Modal onClose={props.onCancel}>
      <EntityDeleter {...props} />
    </Modal>
  );
}
