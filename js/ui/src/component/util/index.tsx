import { Accessor, JSX } from "solid-js";

export function isAccessor<T>(
  value: Accessor<T> | T | null | undefined
): value is Accessor<T> {
  return typeof value === "function";
}

export interface PageTitleProps {
  children: JSX.Element;
}

export function PageTitle(props: PageTitleProps): JSX.Element {
  return <h3 class="title is-3">{props.children}</h3>;
}

export interface GenericPageProps {
  title: JSX.Element;
  children?: JSX.Element;
}

export function GenericPage(props: GenericPageProps): JSX.Element {
  return (
    <div class="container">
      <PageTitle>{props.title}</PageTitle>
      {props.children}
    </div>
  );
}
