import { Link } from "solid-app-router";
import { createSignal, JSX, Show } from "solid-js";
import { useRegistry } from "../../context";
import { ValueMap } from "../../semantic/registry";
import { FACTOR_ID } from "semantic/dist/schema";
import { Button, Buttons } from "../bulma/button";
import { Icon } from "../bulma/icon";
import { EntityPicker } from "./EntityPicker";

export interface EntitySearcherProps {
  autoFocus?: boolean;
  onSelected: (entity: ValueMap) => void;
}

export function EntitySearcher(props: EntitySearcherProps): JSX.Element {
  const reg = useRegistry();

  const [activeItem, setActiveItem] = createSignal<ValueMap | null>(null);

  return (
    <Show
      when={activeItem()}
      keyed
      fallback={
        <EntityPicker
          autoFocus={props.autoFocus}
          renderItem={(item, _index, _onClick) => {
            // TODO: create helper comoponent for button with addons in bulma/button.tsx
            return (
              <div class="mb-2">
                <div class="field has-addons">
                  <p class="control">
                    <Link
                      href={`/entity/${item[FACTOR_ID]}`}
                      class="button"
                      onClick={(e) => {
                        e.preventDefault();
                        setActiveItem(item);
                      }}
                    >
                      {reg.entityTitle(item)}
                    </Link>
                  </p>
                  <p class="control">
                    <Button
                      onClick={() => {
                        props.onSelected(item);
                      }}
                    >
                      <Icon icon="arrowsCross" />
                    </Button>
                  </p>
                </div>
              </div>
            );
          }}
          onSelect={(entity) => {}}
        />
      }
    >
      {(item) => {
        const rendered = reg.renderEditableEntity(item, { preview: false });

        return (
          <div>
            <Buttons>
              <Button onclick={() => setActiveItem(null)}>
                Back to search
              </Button>
            </Buttons>
            <hr />
            {rendered}
          </div>
        );
      }}
    </Show>
  );
}
