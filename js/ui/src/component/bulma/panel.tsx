import { JSX, splitProps } from "solid-js";
import { fontawesomeIconClass, IconName } from "./icon";

export interface PanelProps extends JSX.HTMLAttributes<HTMLElement> {
  heading?: JSX.Element;
}

export function Panel(props: PanelProps): JSX.Element {
  const [local, extra] = splitProps(props, ["heading", "class"]);
  let cls = "panel";
  if ("class" in local) {
    cls += " " + local["class"];
  }

  const heading = props.heading ? (
    <p class="panel-heading">{props.heading}</p>
  ) : null;
  return (
    <nav {...extra} class={cls}>
      {heading}
      {props.children}
    </nav>
  );
}

export interface PanelBlockProps extends JSX.HTMLAttributes<HTMLDivElement> {
  isActive?: boolean;
}

export function PanelBlock(props: PanelBlockProps): JSX.Element {
  const [local, extra] = splitProps(props, ["isActive", "class"]);
  let cls = "panel-block";
  if ("class" in local) {
    cls += " " + local["class"];
  }

  return (
    <div {...extra} class={cls} classList={{ "is-active": props.isActive }}>
      {props.children}
    </div>
  );
}

export interface PanelIconProps {
  icon: IconName;
}

export function PanelIcon(props: PanelIconProps): JSX.Element {
  return (
    <span class="panel-icon" aria-hidden>
      <i class={fontawesomeIconClass(props.icon)} />
    </span>
  );
}
