import { Link } from "solid-app-router";
import { JSX } from "solid-js";
import { useRegistry } from "../../context";
import { Button, Buttons, IconButton } from "../bulma/button";
import { Icon } from "../bulma/icon";
import { GenericPage } from "../util";

export function EntityCreateSelectorPage(): JSX.Element {
  const reg = useRegistry();

  const buttons = reg.schema.db.entities.map((entity) => {
    const title = entity["factor/title"] || entity["factor/ident"];

    return (
      <Link
        class="button is-medium"
        href={`/entity/create/${entity["factor/ident"]}`}
      >
        {title}
      </Link>
    );
  });

  return (
    <GenericPage title="Create">
      <Buttons>
        <Link href="/import" class="button is-large">
          <Icon size="is-medium" icon="globe" />
          <span>Import</span>
        </Link>

        <Link href="/upload" class="button is-large">
          <Icon size="is-medium" icon="upload" />
          <span>Upload</span>
        </Link>
      </Buttons>

      <hr />

      <Buttons>{buttons}</Buttons>
    </GenericPage>
  );
}
