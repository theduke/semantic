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
      <header className="card-header">
        <p className="card-header-title">
          <Link href={props.linkPath} style={{ color: "inherit" }}>
            {props.title}
          </Link>
          <small style={{ "font-weight": "normal", "padding-left": "0.8rem" }}>
            {props.type}
          </small>
        </p>
      </header>
      <div className="card-content">{props.children}</div>
    </div>
  );
}
