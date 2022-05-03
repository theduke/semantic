import { Component, createSignal, JSX, Show } from "solid-js";

import { Link, Route, Router, Routes, useParams } from "solid-app-router";

import { LoginPage } from "./component/LoginPage";
import { SemanticSchema } from "./semantic/core";

import * as context from "./context";
import { Api } from "./api";
import { assertDefined } from ".";
import { BrowsePage } from "./component/entity/BrowsePage";
import { UiRegistry } from "./semantic/registry";
import { EntityPage } from "./component/entity/EntityPage";
import { Navbar, NavbarItem, NavbarItemLink } from "./component/bulma/navbar";
import { basePlugin } from "./semantic/base";
import { Buttons } from "./component/bulma/button";
import { Icon } from "./component/bulma/icon";
import { SettingsPage } from "./component/SettingsPage";
import {
  ROUTE_BROWSE,
  ROUTE_SETTINGS,
  ROUTE_SETTINGS_TAG_MANAGER,
} from "./routing";
import { TagManagerPage } from "./component/tag/TagManager";

export function App(): JSX.Element {
  const [getRegistry, setRegistry] = createSignal<UiRegistry | null>();

  const onLogin = function (schema: SemanticSchema) {
    const reg = new UiRegistry(schema);
    reg.registerPlugin(basePlugin());
    setRegistry(reg);
  };

  return (
    <context.ApiContext.Provider value={new Api()}>
      <Show when={getRegistry()} fallback={<LoginPage onLogin={onLogin} />}>
        <context.UiRegistryContext.Provider
          value={assertDefined(getRegistry())}
        >
          <Router>
            <AppNavbar />
            <Routes>
              <Route path="/" element={BrowsePage} />
              <Route path={ROUTE_BROWSE} element={BrowsePage} />
              <Route path="/entity/:ident" component={EntityPageRoute} />
              <Route path={ROUTE_SETTINGS} component={SettingsPage} />
              <Route
                path={ROUTE_SETTINGS_TAG_MANAGER}
                component={TagManagerPage}
              />
            </Routes>
          </Router>
        </context.UiRegistryContext.Provider>
      </Show>
    </context.ApiContext.Provider>
  );
}

function EntityPageRoute(): JSX.Element {
  const params = useParams<{ ident: string }>();
  return <EntityPage ident={params.ident} />;
}

function AppNavbar(): JSX.Element {
  return (
    <Navbar
      brandLinkContent="Semantic"
      start={
        <>
          <NavbarItemLink href="/browse">Browse</NavbarItemLink>
        </>
      }
      end={
        <>
          <NavbarItem>
            <Buttons>
              <Link
                aria-label="Settings"
                title="Settings"
                class="button"
                href={ROUTE_SETTINGS}
              >
                <Icon icon="userGear" />
              </Link>
              <Link
                class="button"
                title="Log out"
                aria-label="Log out"
                href={ROUTE_SETTINGS_TAG_MANAGER}
              >
                <Icon icon="signOut" />
              </Link>
            </Buttons>
          </NavbarItem>
        </>
      }
    />
  );
}
