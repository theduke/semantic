import { JSX } from "solid-js";

export type TitleSize = "is-1" | "is-2" | "is-3" | "is-4" | "is-5" | "is-6";

export interface SubtitleProps {
  size: TitleSize;
  children: JSX.Element;
}

export function Subtitle(props: SubtitleProps): JSX.Element {
  const cls = "subtitle " + props.size;

  switch (props.size) {
    case "is-1":
      return <h1 class={cls}>{props.children}</h1>;
    case "is-2":
      return <h2 class={cls}>{props.children}</h2>;
    case "is-3":
      return <h3 class={cls}>{props.children}</h3>;
    case "is-4":
      return <h4 class={cls}>{props.children}</h4>;
    case "is-5":
      return <h5 class={cls}>{props.children}</h5>;
    case "is-6":
      return <h6 class={cls}>{props.children}</h6>;
    default:
      return <h1 class={cls}>{props.children}</h1>;
  }
}
