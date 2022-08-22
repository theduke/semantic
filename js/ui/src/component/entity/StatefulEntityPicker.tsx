import { ValueMap } from "semantic/dist/api";
import { Class } from "semantic/dist/core";
import {
  createEffect,
  createSignal,
  JSX,
  Match,
  Show,
  Signal,
  splitProps,
  Switch,
} from "solid-js";
import { useRegistry } from "../../context";
import { Button, Buttons, IconButton } from "../bulma/button";
import { Modal } from "../bulma/modal";
import { EntityCreator } from "./EntityCreator";
import { EntityPicker, EntityPickerProps } from "./EntityPicker";

export interface StatefulEntityPickerProps
  extends Omit<EntityPickerProps, "onSelect"> {
  schema: Class;

  noCreate?: boolean;

  signal?: Signal<ValueMap | undefined>;
  onChange?: (entity: ValueMap | undefined) => void;
}

export function StatefulEntityPicker(
  props: StatefulEntityPickerProps
): JSX.Element {
  const reg = useRegistry();
  const [local, rest] = splitProps(props, ["signal", "onChange", "schema"]);

  const [item, setItem] = props.signal || createSignal<ValueMap | undefined>();
  const [mode, setMode] = createSignal<"empty" | "search" | "create">("empty");

  const clearMode = () => setMode("empty");

  if (props.onChange) {
    const onChange = props.onChange;
    createEffect(() => {
      onChange(item());
    });
  }

  return (
    <div>
      <Show
        when={item()}
        fallback={() => {
          return (
            <Switch>
              <Match when={mode() === "empty"}>
                <Buttons>
                  <IconButton icon="search" onclick={() => setMode("search")}>
                    Find existing entity
                  </IconButton>
                  {!props.noCreate && (
                    <IconButton icon="plus" onclick={() => setMode("create")}>
                      Create new entity
                    </IconButton>
                  )}
                </Buttons>
              </Match>
              <Match when={mode() === "search"}>
                <EntityPicker
                  {...rest}
                  onSelect={(entity) => {
                    setItem(entity);
                    clearMode();
                  }}
                />
                <div class="mt-2"></div>
                <Buttons>
                  <Button onclick={clearMode}>Cancel</Button>
                </Buttons>
              </Match>
              <Match when={mode() === "create"}>
                <Modal isActive={true} onClose={clearMode}>
                  <EntityCreator
                    schema={props.schema}
                    onCancel={clearMode}
                    onPersisted={(data) => {
                      setItem(data);
                      clearMode();
                    }}
                  />
                </Modal>
              </Match>
            </Switch>
          );
        }}
      >
        {(item: ValueMap) => {
          return (
            <div>
              <Button>{reg.entityTitle(item)}</Button>
              <IconButton icon="xmark" onclick={() => setItem(undefined)} />
            </div>
          );
        }}
      </Show>
    </div>
  );
}
