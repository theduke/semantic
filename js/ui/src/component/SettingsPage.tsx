import { Link } from "solid-app-router";
import { JSX } from "solid-js/jsx-runtime";
import {
  ROUTE_SETTINGS_ANALYSE_MEDIA,
  ROUTE_SETTINGS_BLOB_CLEANUP,
  ROUTE_SETTINGS_TAG_MANAGER,
} from "../routing";
import { GenericPage } from "./util";

export function SettingsPage(): JSX.Element {
  return (
    <GenericPage title="Settings">
      <div class="is-flex is-flex-direction-column" style={{ gap: "1rem" }}>
        <Link class="button is-medium" href={ROUTE_SETTINGS_TAG_MANAGER}>
          Tags
        </Link>

        <Link class="button is-medium" href={ROUTE_SETTINGS_BLOB_CLEANUP}>
          Blob Cleanup
        </Link>

        <Link class="button is-medium" href={ROUTE_SETTINGS_ANALYSE_MEDIA}>
          Analyze Media
        </Link>
      </div>
    </GenericPage>
  );
}
