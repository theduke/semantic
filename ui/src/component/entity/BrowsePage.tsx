import {
  createResource,
  createSignal,
  ErrorBoundary,
  For,
  JSX,
  Show,
  Suspense,
} from "solid-js";
import { assertDefined } from "../..";
import { newSelect } from "../../api";
import { useApi, useRegistry } from "../../context";
import { Item, Page, Select } from "../../semantic/core";
import { EntityRenderOpts, UiRegistry } from "../../semantic/registry";
import { GenericPage, PageTitle } from "../util";
import { renderError, SPINNER } from "../util/load";
import { MultiSelectSearch } from "../util/MultiSelect";

export function BrowsePage(): JSX.Element {
  const api = useApi();
  const registry = useRegistry();
  const [select, setSelect] = createSignal<Select>(newSelect());

  const [page] = createResource(select, (select) => {
    return api.select(select);
  });

  const opts: EntityRenderOpts = {
    preview: true,
  };

  return (
    <GenericPage title="Browse">
      <MultiSelectSearch<string>
        search={(term) => {
          return Promise.resolve(["a", "b", "c"]);
        }}
        renderItem={(item) => item}
      />

      <ErrorBoundary fallback={renderError}>
        <Suspense fallback={SPINNER}>
          <Show when={page()}>
            {() => renderItems(registry, assertDefined(page()), opts)}
          </Show>
        </Suspense>
      </ErrorBoundary>
    </GenericPage>
  );
}

function renderItems(
  reg: UiRegistry,
  page: Page<Item>,
  opts: EntityRenderOpts
): JSX.Element {
  return (
    <div class="is-flex is-flex-direction-column mb-4" style={{ gap: "2rem" }}>
      <For each={page.items}>{(item) => reg.renderEntity(item, opts)}</For>
    </div>
  );
}
