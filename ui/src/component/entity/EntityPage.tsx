import { JSX } from "solid-js/jsx-runtime";
import { useApi, useRegistry } from "../../context";
import { IdOrIdent, Item, Select } from "../../semantic/core";
import { EntityRenderOpts } from "../../semantic/registry";
import { EntityLoader } from "./EntityLoader";

export type EntityPageProps =
  | {
      entity: Item;
    }
  | { ident: IdOrIdent };

export function EntityPage(props: EntityPageProps): JSX.Element {
  const api = useApi();
  const registry = useRegistry();

  let content;

  const opts: EntityRenderOpts = {
    preview: false,
  };

  if ("entity" in props) {
    content = registry.renderEditableEntity(props.entity, opts, (_) => {});
  } else {
    content = (
      <EntityLoader ident={props.ident}>
        {(item) => registry.renderEditableEntity(item, opts, (_) => {})}
      </EntityLoader>
    );
  }

  return <div class="container">{content}</div>;
}
