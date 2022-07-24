import { Link } from "solid-app-router";
import { JSX } from "solid-js";
import { useRegistry } from "../../context";
import { Button, Buttons } from "../bulma/button";
import { GenericPage } from "../util";

export function EntityCreateSelectorPage(): JSX.Element {
  const reg = useRegistry();

  const buttons = reg.schema.db.entities.map((entity) => {
    const title = entity["factor/title"] || entity["factor/ident"];

    return (
      <Link class="button" href={`/entity/create/${entity["factor/ident"]}`}>
        {title}
      </Link>
    );
  });

  return (
    <GenericPage title="Create Entity">
      <Buttons>{buttons}</Buttons>
    </GenericPage>
  );
}
