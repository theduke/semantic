import { JSX } from "solid-js";

export function Box(props: JSX.HTMLAttributes<HTMLDivElement>): JSX.Element {
  const cls = 'box' + (props.class ? ' ' + props.class : '');
  return <div {...props} class={cls} />;
}
