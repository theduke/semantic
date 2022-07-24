import { Link } from "solid-app-router";
import { useNavigate } from "solid-app-router";
import { createSignal, JSX, Show } from "solid-js";
import { Portal } from "solid-js/web";
import { useRegistry } from "../../context";
import { ValueMap } from "../../semantic/registry";
import { FACTOR_ID } from "../../semantic/schema";
import { Box } from "solid-bulma";
import { Button, Buttons } from "../bulma/button";
import { Icon } from "../bulma/icon";
import { Modal } from "../bulma/modal";
import { EntityPicker } from "./EntityPicker";

export interface EntitySearcherProps {
  onSelected: (entity: ValueMap) => void;
}

export function EntitySearcher(props: EntitySearcherProps): JSX.Element {
  const reg = useRegistry();

  const [activeItem, setActiveItem] = createSignal<ValueMap | null>(null);

  return (
    <Show
      when={activeItem()}
      fallback={
        <EntityPicker
          autoFocus={true}
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

export function EntitySearcherModalToggle(): JSX.Element {
  const [active, setActive] = createSignal(false);

  const navigate = useNavigate();

  return (
    <div style={{ display: "flex", "align-items": "center" }}>
      <Button
        size="is-normal"
        onClick={() => {
          setActive(true);
        }}
      >
        <Icon icon="search" />
      </Button>

      <Show when={active()}>
        <Portal>
          <Modal onClose={() => setActive(false)}>
            <Box>
              <EntitySearcher
                onSelected={(entity) => {
                  setActive(false);
                  navigate(`/entity/${entity[FACTOR_ID]}`);
                }}
              />
            </Box>
          </Modal>
        </Portal>
      </Show>
    </div>
  );
}
