import { createSignal, JSX, Show } from "solid-js";

import { Link, Route, Router, Routes, useParams } from "solid-app-router";

import { LoginPage } from "./component/LoginPage";
import { SemanticSchema } from "semantic/dist/core";

import * as context from "./context";
import { Api } from "semantic/dist/api";
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
  ROUTE_SETTINGS,
  ROUTE_SETTINGS_ANALYSE_MEDIA,
  ROUTE_SETTINGS_BLOB_CLEANUP,
  ROUTE_SETTINGS_TAG_MANAGER,
} from "./routing";
import { TagManagerPage } from "./component/tag/TagManager";
import { EntityCreateSelectorPage } from "./component/entity/EntityCreateSelectorPage";
import { EntityCreatePage } from "./component/entity/EntityCreatePage";
import { EntitySearcherModalToggle } from "./component/entity/EntitySearcherModalToggle";
import { UploadPage } from "./component/upload/UploadPage";
import { ImportPage } from "./component/imports/ImportPage";
import { PlayPage } from "./component/play/PlayPage";
import { BlobCleanupPage } from "./component/settings/blob_cleanup";
import { MediaAnalyzerPage } from "./component/settings/media_analyzer";

export function App(): JSX.Element {
  const [getRegistry, setRegistry] = createSignal<UiRegistry | null>();

  const onLogin = function (schema: SemanticSchema) {
    const reg = new UiRegistry(schema);
    reg.registerPlugin(basePlugin());
    setRegistry(reg);
  };

  const serverUrl = "/";

  return (
    <context.ApiContext.Provider value={new Api(serverUrl)}>
      <Show when={getRegistry()} fallback={<LoginPage onLogin={onLogin} />}>
        <context.UiRegistryContext.Provider
          value={assertDefined(getRegistry())}
        >
          <Router>
            <div
              style={{
                display: "flex",
                "flex-direction": "column",
                height: "100%",
              }}
            >
              <AppNavbar />
              <Routes>
                <Route path="/entity/*ident" component={EntityPageRoute} />
                <Route
                  path="/entity/create"
                  component={EntityCreateSelectorPage}
                />
                <Route
                  path="/entity/create/*ident"
                  component={() => {
                    const params = useParams<{ ident: string }>();
                    return <EntityCreatePage entityType={params.ident} />;
                  }}
                />
                <Route path="/upload" component={UploadPage} />
                <Route path="/play" component={PlayPage} />
                <Route path="/import" component={ImportPage} />
                <Route path={ROUTE_SETTINGS} component={SettingsPage} />
                <Route
                  path={ROUTE_SETTINGS_TAG_MANAGER}
                  component={TagManagerPage}
                />
                <Route
                  path={ROUTE_SETTINGS_BLOB_CLEANUP}
                  component={BlobCleanupPage}
                />
                <Route
                  path={ROUTE_SETTINGS_ANALYSE_MEDIA}
                  component={MediaAnalyzerPage}
                />
                <Route path="/" component={BrowsePage} />
              </Routes>
            </div>
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
      brandExtra={[<EntitySearcherModalToggle keyboard />]}
      start={
        <>
          <NavbarItemLink href="/">Browse</NavbarItemLink>
          <NavbarItemLink href="/entity/create">Create</NavbarItemLink>
          <NavbarItemLink href="/play">Play</NavbarItemLink>
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
                <Icon icon="gear" />
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
