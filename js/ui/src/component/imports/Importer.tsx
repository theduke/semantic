import { throttle } from "lodash";
import { Api, newSelect, ValueMap } from "semantic/dist/api";
import { FetchUrlJob } from "semantic/dist/core";
import { exprAttr, exprIn, exprList, exprLiteral } from "semantic/dist/db";
import { BaseEntity, FACTOR_TYPE, SEMANTIC_URL } from "semantic/dist/schema";
import { Link } from "solid-app-router";
import { Box } from "solid-bulma";
import {
  createResource,
  createSignal,
  ErrorBoundary,
  For,
  JSX,
  Match,
  Show,
  Signal,
  Switch,
} from "solid-js";
import { useApi, useRegistry } from "../../context";
import { entityLinkPath } from "../../semantic";
import { Button, Buttons } from "../bulma/button";
import {
  Notification,
  NotificationError,
  NotificationWarning,
} from "../bulma/notification";
import { renderEntityTable } from "../entity";
import { EntityBox } from "../entity/EntityBox";
import { createForm } from "../form";
import { CheckboxField } from "../form/CheckboxField";
import { InputField } from "../form/InputField";
import { loadAsError, LoadState, renderError, SPINNER } from "../util/load";
import zod from "zod";

interface FormValues {
  url: string;
  importMedia: boolean;
  skipExisting: boolean;
}

type ItemId = number;

export interface ImportSettings {
  url?: string | null;
  importMedia?: boolean | null;
  skipExisting?: boolean | null;
}

export const validateImportSettings = zod.object({
  url: zod.string().optional(),
  importMedia: zod.boolean().optional(),
});

interface QueueItem {
  id: ItemId;
  data: ValueMap;
  oldEntity: BaseEntity | null;
  loader: Signal<LoadState<ValueMap>>;
}

export interface ImporterProps {
  initialSettings?: ImportSettings;
  onSettingsChanged?: (settings: ImportSettings) => void;
}

export function Importer(props: ImporterProps): JSX.Element {
  const api = useApi();
  const reg = useRegistry();

  const initialValues = {
    url: props.initialSettings?.url ?? "",
    importMedia: props.initialSettings?.importMedia ?? true,
    skipExisting: props.initialSettings?.skipExisting ?? false,
  };
  let initialQuery = undefined;
  if (initialValues.url) {
    initialQuery = { url: initialValues.url };
  }

  const [values, setValues] = createSignal<FormValues>(initialValues);
  const [query, setQuery] = createSignal<FetchUrlJob | undefined>(initialQuery);

  const [queueItems, setQueueItems] = createSignal<QueueItem[]>([]);
  const [importedItems, setImportedItems] = createSignal<BaseEntity[]>([]);

  const form = createForm<FormValues>({
    initialValues: initialValues,
    onValid: (values) => {
      onValueChange(values);
    },
  });

  const onValueChange = throttle((values: FormValues) => {
    const url = values.url;
    if (url && query()?.url !== url) {
      setQuery({ url });
    } else if (values.skipExisting === true) {
      setQueueItems((old) => old.filter((item) => !item.oldEntity));
    }
  }, 500);

  let nextItemId = 0;

  const [results, { mutate }] = createResource<true | null, FetchUrlJob>(
    query,
    async (query): Promise<true | null> => {
      if (!(query && query.url)) {
        return null;
      }
      let items = await loadPreview(api, query, nextItemId);
      nextItemId += items.length;
      console.debug({ skip: form.state.fields.skipExisting?.value });
      if (form.state.fields.skipExisting?.value ?? false) {
        items = items.filter((item) => !item.oldEntity);
      }

      setQueueItems(items);
      props.onSettingsChanged?.({
        url: query.url,
        importMedia: values().importMedia,
      });

      return true;
    },
    { initialValue: null }
  );

  const doImport = async (originalItem: QueueItem) => {
    const item: QueueItem | undefined = queueItems().find(
      (i) => i.id === originalItem.id
    );
    if (!item || item.loader[0]().state === "loading") {
      return;
    }
    const url = item.data[SEMANTIC_URL];
    if (typeof url !== "string") {
      return;
    }
    item.loader[1]({ state: "loading" });
    let out;
    try {
      out = await api.import({ url, import_media: values().importMedia });
    } catch (error: any) {
      item.loader[1]({ state: "error", error: error.toString() });
      return;
    }
    const finalItem = out.items.find((i) => i.data[SEMANTIC_URL] === url)?.data;
    if (finalItem) {
      setQueueItems((items) => items.filter((i) => i.id !== item.id));
      setImportedItems((items) => [...items, finalItem as BaseEntity]);
    } else {
      item.loader[1]({
        state: "error",
        error: "Import failed - item url was not found in import result.",
      });
    }
  };

  const discardItem = (originalItem: QueueItem) => {
    const item = queueItems().find((i) => i.id === originalItem.id);
    if (!item) {
      return;
    }
    if (item.loader[0]().state === "loading") {
      return;
    }
    setQueueItems((items) => items.filter((i) => i.id !== item.id));
  };

  const urlField = form.field("url");

  return (
    <div>
      <Box>
        <form
          onsubmit={(e) => {
            e.preventDefault();
            e.stopPropagation();
          }}
        >
          <InputField
            mode="oninput"
            icon="search"
            field={urlField}
            label={null}
            help={
              results.loading && urlField.get().value !== query()?.url ? (
                <span class="has-text-info">
                  <b>Pending...</b>
                </span>
              ) : (
                "Enter a url to import."
              )
            }
            placeholder="https://..."
          />
          <CheckboxField
            field={form.field("importMedia")}
            label={null}
            checkboxLabel={"Import associated media"}
          />
          <CheckboxField
            field={form.field("skipExisting")}
            label={null}
            checkboxLabel={"Hide already imported entities"}
          />
        </form>
      </Box>

      <div>
        <ErrorBoundary fallback={renderError}>
          <Switch>
            <Match when={results.loading}>{SPINNER}</Match>
            <Match when={results() === null}>
              <Notification>Enter a url...</Notification>
            </Match>
            <Match when={results() === true}>
              <Show
                when={queueItems().length > 0}
                fallback={<Notification>No items found</Notification>}
              >
                <div
                  style={{
                    display: "flex",
                    "flex-direction": "column",
                    gap: "1rem",
                  }}
                >
                  <For each={queueItems()}>
                    {(item) => {
                      const data = item.data;

                      const ident = data[FACTOR_TYPE];
                      const contentRender = reg.entityContentRenderers[ident];
                      console.debug({ contentRender });
                      let content;
                      if (contentRender) {
                        content = contentRender(data, { preview: true });
                      } else {
                        content = renderEntityTable(reg, data);
                      }
                      const title = reg.entityTitle(data);
                      return (
                        <EntityBox title={title}>
                          <Buttons>
                            <Show when={item.data[SEMANTIC_URL]}>
                              <Button
                                loading={item.loader[0]().state === "loading"}
                                onClick={() => {
                                  doImport(item);
                                }}
                              >
                                Import
                              </Button>
                            </Show>
                            <Button
                              onClick={() => {
                                discardItem(item);
                              }}
                            >
                              Discard
                            </Button>
                          </Buttons>

                          <Show when={item.oldEntity}>
                            {(old) => {
                              return (
                                <NotificationWarning>
                                  Already imported:{" "}
                                  <Link href={entityLinkPath(old)}>
                                    {reg.entityTitle(old)}
                                  </Link>
                                </NotificationWarning>
                              );
                            }}
                          </Show>

                          <Show when={loadAsError(item.loader[0]())}>
                            {(err) => (
                              <NotificationError>{err}</NotificationError>
                            )}
                          </Show>

                          <hr />

                          {content}
                        </EntityBox>
                      );
                    }}
                  </For>
                </div>
              </Show>
            </Match>
          </Switch>
        </ErrorBoundary>

        <Show when={importedItems().length > 0}>
          <h4 class="title is-4 mt-4">Imported</h4>

          <Buttons>
            <Button
              onClick={() => {
                setImportedItems([]);
              }}
            >
              Clear
            </Button>
          </Buttons>

          <div
            style={{ display: "flex", "flex-direction": "column", gap: "1rem" }}
          >
            <For each={importedItems()}>
              {(item) =>
                reg.renderEntity(item, { preview: true, allowEdit: true })
              }
            </For>
          </div>
        </Show>
      </div>
    </div>
  );
}

async function loadPreview(api: Api, query: FetchUrlJob, firstItemId: number) {
  const out = await api.fetchUrl(query);
  // Find existing.
  const urls: string[] = out.items
    .map((item) => item.data[SEMANTIC_URL])
    .filter((x) => !!x);
  const existingEntities = await api.select({
    ...newSelect(),
    filter: exprIn(exprAttr(SEMANTIC_URL), exprList(urls.map(exprLiteral))),
    limit: Math.max(urls.length * 2, 500) as any,
  });

  let nextItemId = firstItemId;

  const items = out.items.map(
    (item): QueueItem => ({
      id: nextItemId++,
      data: item.data as ValueMap,
      loader: createSignal({ state: "idle" }),
      oldEntity: existingEntities.find(
        (entity) => entity[SEMANTIC_URL] === item.data[SEMANTIC_URL]
      ) as BaseEntity | null,
    })
  );
  return items;
}
