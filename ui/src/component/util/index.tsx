import { JSX } from "solid-js";

export interface PageTitleProps {
  children: JSX.Element;
}

export function PageTitle(props: PageTitleProps): JSX.Element {
  return <h2 class="title is-2">{props.children}</h2>;
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
