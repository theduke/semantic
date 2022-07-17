import { Link } from "solid-app-router";
import { JSX } from "solid-js/jsx-runtime";

export interface EntityBoxProps {
  linkPath: string;
  title: JSX.Element;
  type?: string;
  children?: JSX.Element;
}

export function EntityBox(props: EntityBoxProps): JSX.Element {
  return (
    <div class="card">
      <header class="card-header">
        <p class="card-header-title">
          <Link href={props.linkPath} style={{ color: "inherit" }}>
            {props.title}
          </Link>
          <small style={{ "font-weight": "normal", "padding-left": "0.8rem" }}>
            {props.type}
          </small>
        </p>
      </header>
      <div class="card-content">{props.children}</div>
    </div>
  );
}
