import { untrack } from "solid-js";
import { JSX } from "solid-js/jsx-runtime";
import { useRegistry } from "../../context";
import { IdOrIdent, Item } from "semantic/dist/core";
import { EntityRenderOpts } from "../../semantic/registry";
import { EntityLoader } from "./EntityLoader";

export type EntityPageProps =
  | {
      entity: Item;
    }
  | { ident: IdOrIdent };

export function EntityPage(props: EntityPageProps): JSX.Element {
  const registry = useRegistry();

  const opts: EntityRenderOpts = {
    preview: false,
    allowEdit: true,
    allowDelete: true,
  };

  let content: JSX.Element;
  if ("entity" in props) {
    content = registry.renderEditableEntity(props.entity, opts, (_) => {});
  } else {
    content = (
      <EntityLoader ident={props.ident}>
        {(item) => {
          return untrack(() => {
            console.debug("rendering editable entity");
            return registry.renderEditableEntity(item, opts, (_) => {});
          });
        }}
      </EntityLoader>
    );
  }

  return <div class="container">{content}</div>;
}
