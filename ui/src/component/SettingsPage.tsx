import { Link } from "solid-app-router";
import { JSX } from "solid-js/jsx-runtime";
import { Button } from "./bulma/button";
import { GenericPage } from "./util";

export function SettingsPage(): JSX.Element {
  return (
    <GenericPage title="Settings">
      <div class="is-flex is-flex-direction-column" style={{ gap: "4rem" }}>
        <Link class="button is-large" href="/settings/tags">
          Tags
        </Link>
      </div>
    </GenericPage>
  );
}
