import { JSX } from "solid-js/jsx-runtime";
import { Color } from ".";

export interface NotificationProps {
  color?: Color;
  isLight?: boolean;
  children: JSX.Element;
}

export function Notification(props: NotificationProps): JSX.Element {
  let cls = "notificaton";
  if (props.color) {
    cls += " " + props.color;
  }
  if (props.isLight) {
    cls += " " + "is-light";
  }

  return <div class={cls}>{props.children}</div>;
}

export function NotificationError(
  props: Omit<NotificationProps, "color" | "class">
): JSX.Element {
  return <Notification {...props} color="is-danger" />;
}

export function NotificationWarning(
  props: Omit<NotificationProps, "color">
): JSX.Element {
  return <Notification {...props} color="is-warning" />;
}
