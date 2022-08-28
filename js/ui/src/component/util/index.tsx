import { Accessor, ErrorBoundary, JSX, ParentProps } from "solid-js";
import { NotificationError } from "../bulma/notification";

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
    <div class="container pb-6" style={{width: '100%'}}>
      <PageTitle>{props.title}</PageTitle>
      {props.children}
    </div>
  );
}

export function prettyPrintByteSize(size: number): string {
  if (size < 1000) {
    return `${size}B`;
  } else if (size < 1000 * 1000) {
    return `${(size / 1000).toFixed()}Kb`;
  } else if (size < 1000 * 1000 * 1000) {
    return `${(size / 1000 / 1000).toFixed()}Mb`;
  } else {
    return `${(size / 1000 / 1000 / 1000).toFixed(2)}Gb`;
  }
}

export function NotificationErrorBoundary(props: ParentProps): JSX.Element {
  return (
    <ErrorBoundary
      fallback={(error: any) => (
        <NotificationError>{error.toString()}</NotificationError>
      )}
    >
      {props.children}
    </ErrorBoundary>
  );
}

export function tryParseFloat(value: string): number | null {
  try {
    return parseFloat(value);
  } catch {
    return null;
  }
}
