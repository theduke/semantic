import { JSX, splitProps } from "solid-js";
import { Color } from ".";

export type ButtonsSize = "are-small" | "are-medium" | "are-large";

export interface ButtonsProps {
  children?: JSX.Element;
  size?: ButtonsSize;
  // If true, the buttons are grouped together without spacing.
  attach?: boolean;
}

export function Buttons(props: ButtonsProps): JSX.Element {
  let cls = "buttons";
  if (props.size) {
    cls += " " + props.size;
  }
  if (props.attach) {
    cls += " has-addons";
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
  ]);
  let cls = "button";
  if (localProps.size) {
    cls += " " + props.size;
  }
  if (localProps.color) {
    cls += " " + props.color;
  }
  if (localProps.fullWidth) {
    cls += " is-fullwidth";
  }
  if (localProps.outlined) {
    cls += " is-outlined";
  }
  if (localProps.inverted) {
    cls += " is-inverted";
  }
  if (localProps.rounded) {
    cls += " is-rounded";
  }
  if (localProps.loading) {
    cls += " is-loading";
  }
  if (localProps.isStatic) {
    cls += " is-static";
  }
  if (localProps.selected) {
    cls += " is-selected";
  }
  if (localProps.class) {
    cls += " " + localProps.class;
  }

  return <button {...extraProps} class={cls}></button>;
}
