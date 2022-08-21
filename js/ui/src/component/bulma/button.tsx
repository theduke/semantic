import { JSX, splitProps } from "solid-js";
import { Color } from ".";
import { isAccessor } from "../util";
import { Icon, IconName, IconSize } from "./icon";

export type ButtonsSize = "are-small" | "are-medium" | "are-large";

export interface ButtonsProps extends JSX.HTMLAttributes<HTMLDivElement> {
  size?: ButtonsSize;
  // If true, the buttons are grouped together without spacing.
  attach?: boolean;
  hasAddons?: boolean;
}

export function Buttons(props: ButtonsProps): JSX.Element {
  let cls = "buttons";
  if (props.size) {
    cls += " " + props.size;
  }
  if (props.attach) {
    cls += " has-addons";
  }
  if (props.class) {
    cls += " " + props.class;
  }
  return <div class={cls}>{props.children}</div>;
}

export type ButtonSize = "is-small" | "is-normal" | "is-medium" | "is-large";

export interface ButtonProps
  extends JSX.ButtonHTMLAttributes<HTMLButtonElement> {
  size?: ButtonSize;
  color?: Color;
  fullWidth?: boolean;
  outlined?: boolean;
  inverted?: boolean;
  rounded?: boolean;
  loading?: boolean;
  isActive?: boolean;
  isStatic?: boolean;
  // TODO: is this only valid in in a "buttons" wrapper with "has-addons"?
  selected?: boolean;
}

export function Button(props: ButtonProps): JSX.Element {
  const [localProps, extraProps] = splitProps(props, [
    "size",
    "color",
    "fullWidth",
    "outlined",
    "inverted",
    "rounded",
    "loading",
    "isStatic",
    "selected",
    "class",
    "isActive",
  ]);

  return (
    <button
      class={
        "button" +
        (localProps.size ? " " + localProps.size : "") +
        (localProps.color ? " " + localProps.color : "") +
        (localProps.class ? " " + localProps.class : "")
      }
      classList={{
        "is-active": localProps.isActive,
        "is-outlined": localProps.outlined,
        "is-loading": localProps.loading,
        "is-static": localProps.isStatic,
        "is-selected": localProps.selected,
        "is-rounded": localProps.rounded,
        "is-inverted": localProps.inverted,
        "is-fullwidth": localProps.fullWidth,
      }}
      {...extraProps}
    >
      {props.children}
    </button>
  );
}

export interface IconButtonProps extends ButtonProps {
  icon: IconName;
}

export function IconButton(props: IconButtonProps): JSX.Element {
  const [local, rest] = splitProps(props, ["icon", "children"]);

  let iconSize: IconSize | undefined;
  switch (props.size) {
    case "is-small":
      iconSize = "is-small";
      break;
    case "is-large":
      iconSize = "is-medium";
      break;
  }

  return (
    <Button {...rest}>
      <Icon size={iconSize} icon={local.icon} />
      {props.children ? <span>{props.children}</span> : null}
    </Button>
  );
}
