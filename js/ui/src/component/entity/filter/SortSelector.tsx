import { Attribute, Order, Sort } from "semantic/dist/core";
import { FACTOR_IDENT, FACTOR_TITLE, Ident } from "semantic/dist/schema";
import { createSignal, JSX, Match, Show, Switch } from "solid-js";
import { assertDefined } from "../../..";
import { useRegistry } from "../../../context";
import { Button, Buttons } from "../../bulma/button";

export interface SortSelectorProps {
  onSelected: (selection: [Attribute, Order] | null) => void;
  initialSelection?: [Ident, Order];
}

export function SortSelector(props: SortSelectorProps): JSX.Element {
  const reg = useRegistry();
  const attrs = Object.values(reg.attrs);
  attrs.sort((a, b) => {
    const x = a[FACTOR_IDENT] ?? "";
    const y = b[FACTOR_IDENT] ?? "";
    if (x < y) {
      return -1;
    } else if (x > y) {
      return 1;
    } else {
      return 0;
    }
  });

  let initial: { attr: Attribute | null; order: Order | null } = {
    attr: null,
    order: null,
  };
  if (props.initialSelection) {
    const i = props.initialSelection;
    const a = attrs.find((a) => a[FACTOR_IDENT] === i[0]);
    if (a) {
      initial = { attr: a, order: i[1] };
    }
  }
  console.debug({ initial: props.initialSelection, built: initial });

  const [state, setState] = createSignal<{
    attr: Attribute | null;
    order: Order | null;
  }>(initial);

  return (
    <div>
      <Switch>
        <Match when={!state().attr}>
          <Buttons>
            {attrs.map((attr) => (
              <Button
                title={attr[FACTOR_IDENT]}
                size="is-small"
                onclick={() => setState({ attr, order: null })}
              >
                {attr[FACTOR_TITLE] || attr[FACTOR_IDENT]}
              </Button>
            ))}
          </Buttons>
        </Match>
        <Match when={state().order === null}>
          {() => {
            const attr = assertDefined(state().attr);
            return (
              <div class="is-flex">
                <Button disabled class="mr-4">
                  {attr[FACTOR_TITLE]}
                </Button>

                <Buttons attach>
                  <Button
                    onclick={() => {
                      setState({ attr, order: "Asc" });
                      props.onSelected([attr, "Asc"]);
                    }}
                  >
                    Asc
                  </Button>
                  <Button
                    onclick={() => {
                      setState({ attr, order: "Desc" });
                      props.onSelected([attr, "Desc"]);
                    }}
                  >
                    Desc
                  </Button>
                </Buttons>
              </div>
            );
          }}
        </Match>
        <Match when={state().attr && state().order}>
          <Buttons>
            <Button disabled>
              {assertDefined(state().attr)[FACTOR_TITLE] ||
                assertDefined(state().attr)[FACTOR_IDENT]}
              {" | "}
              {assertDefined(state().order)}
            </Button>
            <Button
              onclick={() => {
                setState({ attr: null, order: null });
                props.onSelected(null);
              }}
            >
              Clear
            </Button>
          </Buttons>
        </Match>
      </Switch>
    </div>
  );
}
