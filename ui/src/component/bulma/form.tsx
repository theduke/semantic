import { JSX, PropsWithChildren } from "solid-js";

export function Field(props: JSX.HTMLAttributes<HTMLDivElement>): JSX.Element {
  let cls = "field";
  if (props.class) {
    cls += " " + props.class;
  }
  return <div {...props} class={cls} />;
}

export function Label(props: PropsWithChildren): JSX.Element {
  return <label class="label">{props.children}</label>;
}

export interface ControlProps extends JSX.HTMLAttributes<HTMLDivElement> {}

export function Control(props: ControlProps): JSX.Element {
  let cls = "control";
  if (props.class) {
    cls += " " + props.class;
  }

  return <div {...props} class={cls} />;
}
