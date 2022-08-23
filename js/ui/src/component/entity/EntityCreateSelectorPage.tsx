import { FACTOR_IDENT, FACTOR_TITLE } from "semantic/dist/schema";
import { Link } from "solid-app-router";
import { JSX } from "solid-js";
import { useRegistry } from "../../context";
import { Buttons } from "../bulma/button";
import { Icon } from "../bulma/icon";
import { GenericPage } from "../util";

export function EntityCreateSelectorPage(): JSX.Element {
  const reg = useRegistry();

  const classes = [...reg.schema.db.classes];
  classes.sort((a, b) => {
    const an = a[FACTOR_TITLE] || a[FACTOR_IDENT];
    const bn = b[FACTOR_TITLE] || b[FACTOR_IDENT];

    if (an < bn) {
      return -1;
    } else if (an > bn) {
      return 1;
    } else {
      return 0;
    }
  });

  const buttons = classes
    .filter((entity) => !entity[FACTOR_IDENT]?.startsWith("factor/"))
    .map((entity) => {
      const title = entity[FACTOR_TITLE] || entity[FACTOR_IDENT];

      return (
        <Link
          class="button is-medium"
          title={entity[FACTOR_IDENT]}
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
