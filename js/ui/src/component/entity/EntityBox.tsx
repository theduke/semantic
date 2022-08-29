import { Link } from "solid-app-router";
import { createSignal, For, Show } from "solid-js";
import { JSX } from "solid-js/jsx-runtime";
import { Button } from "../bulma/button";
import { Icon, IconName } from "../bulma/icon";

export interface EntityActionRenderProps {
  close: (action: EntityAction) => void;
}

export interface EntityAction {
  label: string;
  icon: IconName;
  replacesContent: boolean;
  render: (opts: EntityActionRenderProps) => JSX.Element;
  renderAction?: () => JSX.Element;
}

export interface EntityBoxProps {
  linkPath?: string;
  title: JSX.Element;
  type?: string;
  actions?: EntityAction[];
  children?: JSX.Element;
}

export function EntityBox(props: EntityBoxProps): JSX.Element {
  const [activeAction, setActiveAction] = createSignal<{
    index: number;
  } | null>(null);
  const actionRenderProps: EntityActionRenderProps = {
    close: () => {
      setActiveAction(null);
    },
  };

  let actions: JSX.Element = null;
  if (props.actions && props.actions.length > 0) {
    actions = (
      <div style={{ display: "flex", "align-items": "center" }}>
        <For each={props.actions}>
          {(action, indexSignal) => {
            if (action.renderAction) {
              return action.renderAction();
            }

            const index = indexSignal();
            return (
              <Button
                title={action.label}
                isActive={activeAction()?.index === indexSignal()}
                size="is-small"
                onclick={() => {
                  if (activeAction()?.index === indexSignal()) {
                    setActiveAction(null);
                  } else {
                    setActiveAction({ index });
                  }
                }}
              >
                <Icon icon={action.icon} />
              </Button>
            );
          }}
        </For>
      </div>
    );
  }

  return (
    <div class="card">
      <header class="card-header">
        <p class="card-header-title" style={{ "flex-grow": 0 }}>
          {props.linkPath ? (
            <Link href={props.linkPath} style={{ color: "inherit" }}>
              {props.title}
            </Link>
          ) : (
            <span>{props.title}</span>
          )}
          <small style={{ "font-weight": "normal", "padding-left": "0.8rem" }}>
            {props.type}
          </small>
        </p>
        {actions}
      </header>
      <div
        class="card-content"
        style={{ width: "100%", "max-width": "100%", "overflow-y": "scroll" }}
      >
        <Show when={activeAction()} fallback={() => props.children}>
          {(index) => {
            const action = props.actions?.[index.index];
            if (!action) {
              throw new Error("invalid action index");
            }

            if (action.replacesContent) {
              return action.render(actionRenderProps);
            } else {
              return [props.children, action.render(actionRenderProps)];
            }
          }}
        </Show>
      </div>
    </div>
  );
}
