import { ParentProps } from "solid-js";
import { JSX } from "solid-js/jsx-runtime";

export function Block(props: ParentProps): JSX.Element {
  return <div class="block">{props.children}</div>
}
