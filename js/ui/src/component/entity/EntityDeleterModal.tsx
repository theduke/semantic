import { JSX } from "solid-js";
import { BaseEntity } from "semantic/dist/schema";
import { Modal } from "../bulma/modal";
import { EntityDeleter, EntityDeleterProps } from "./EntityDeleter";

export type EntityDeleterModalProps<E extends BaseEntity> =
  EntityDeleterProps<E>;

export function EntityDeleterModal<E extends BaseEntity>(
  props: EntityDeleterModalProps<E>
): JSX.Element {
  return (
    <Modal onClose={props.onCancel}>
      <EntityDeleter {...props} />
    </Modal>
  );
}
