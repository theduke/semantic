import { FACTOR_ID } from "semantic/dist/schema";
import { useNavigate } from "solid-app-router";
import { Box } from "solid-bulma";
import { createSignal, JSX, onCleanup, Show } from "solid-js";
import { Portal } from "solid-js/web";
import { Button } from "../bulma/button";
import { Icon } from "../bulma/icon";
import { Modal } from "../bulma/modal";
import { EntitySearcher } from "./search";

export interface EntitySearcherModalToggleProps {
  // If true, listens to CTRL+S keypresses to show / hide the modal.
  keyboard: boolean;
}

export function EntitySearcherModalToggle(
  props: EntitySearcherModalToggleProps
): JSX.Element {
  const [active, setActive] = createSignal(false);

  const navigate = useNavigate();

  if (props.keyboard) {
    const handler = (e: KeyboardEvent) => {
      if (e.key === "s" && e.altKey) {
        e.stopPropagation();
        e.preventDefault();
        setActive(!active);
      }
    };
    window.addEventListener("keyup", handler);
    onCleanup(() => {
      window.removeEventListener("keyup", handler);
    });
  }

  return (
    <div style={{ display: "flex", "align-items": "center" }}>
      <Button
        title="Quick Search"
        size="is-normal"
        onClick={() => {
          setActive(true);
        }}
      >
        <Icon icon="search" />
      </Button>

      <Show when={active()} keyed>
        {(_) => (
          <Portal>
            <Modal onClose={() => setActive(false)}>
              <Box>
                <EntitySearcher
                  autoFocus
                  onSelected={(entity) => {
                    setActive(false);
                    navigate(`/entity/${entity[FACTOR_ID]}`);
                  }}
                />
              </Box>
            </Modal>
          </Portal>
        )}
      </Show>
    </div>
  );
}
