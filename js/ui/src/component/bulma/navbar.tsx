import { Link, LinkProps } from "solid-app-router";
import { Accessor, createSignal, JSX, ParentProps } from "solid-js";

export interface NavbarBrandProps {
  children: JSX.Element;
  onBurgerToggle: () => void;
}

export function NavbarBrand(props: NavbarBrandProps): JSX.Element {
  return (
    <div class="navbar-brand">
      {props.children}

      <a
        role="button"
        class="navbar-burger"
        aria-label="menu"
        aria-expanded="false"
        onclick={(e) => {
          e.stopPropagation();
          props.onBurgerToggle();
        }}
      >
        <span aria-hidden="true"></span>
        <span aria-hidden="true"></span>
        <span aria-hidden="true"></span>
      </a>
    </div>
  );
}

export interface NavbarMenuProps {
  children: JSX.Element;
  isActive: Accessor<boolean>;
}

export function NavbarMenu(props: NavbarMenuProps): JSX.Element {
  return (
    <div class="navbar-menu" classList={{ "is-active": props.isActive() }}>
      {props.children}
    </div>
  );
}

export function NavbarStart(props: ParentProps): JSX.Element {
  return <div class="navbar-start">{props.children}</div>;
}

export function NavbarEnd(props: ParentProps): JSX.Element {
  return <div class="navbar-end">{props.children}</div>;
}

export function NavbarItem(props: ParentProps): JSX.Element {
  return <div class="navbar-item">{props.children}</div>;
}

export function NavbarItemLink(props: LinkProps): JSX.Element {
  return (
    <Link class="navbar-item" {...props}>
      {props.children}
    </Link>
  );
}

export function NavbarLink(props: LinkProps): JSX.Element {
  return (
    <Link class="navbar-link" {...props}>
      {props.children}
    </Link>
  );
}

export interface NavbarProps {
  brandLinkContent: JSX.Element;
  brandLinkHref?: string | null;
  brandExtra?: JSX.Element;

  start?: JSX.Element;
  end?: JSX.Element;
}

export function Navbar(props: NavbarProps): JSX.Element {
  const [isActive, setIsActive] = createSignal<boolean>(false);

  return (
    <div class="navbar" role="navigation" aria-label="main navigation">
      <NavbarBrand onBurgerToggle={() => setIsActive((old) => !old)}>
        <Link class="navbar-item" href={props.brandLinkHref ?? "/"}>
          {props.brandLinkContent}
        </Link>
        {props.brandExtra}
      </NavbarBrand>

      <NavbarMenu isActive={isActive}>
        <NavbarStart>{props.start}</NavbarStart>
        <NavbarEnd>{props.end}</NavbarEnd>
      </NavbarMenu>
    </div>
  );
}

export function NavbarDivier(): JSX.Element {
  return <hr class="navbar-divier" />;
}
