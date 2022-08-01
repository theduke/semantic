import { throttle } from "lodash";
import { newSelect, ValueMap } from "semantic/dist/api";
import { FetchUrlJob, FetchUrlOutput, ImportOutput } from "semantic/dist/core";
import { exprAttr, exprIn, exprList, exprLiteral } from "semantic/dist/db";
import { BaseEntity, FACTOR_TYPE, SEMANTIC_URL } from "semantic/dist/schema";
import { Link } from "solid-app-router";
import { Box } from "solid-bulma";
import {
  createResource,
  createSignal,
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
import { loadAsError, loadAsSuccess, LoadState, SPINNER } from "../util/load";
import zod from "zod";

interface FormValues {
  url: string;
  importMedia: boolean;
}

type ItemId = number;

export interface ImportSettings {
  url?: string | null;
  importMedia?: boolean | null;
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

interface State {
  state: FetchUrlOutput;
  items: QueueItem[];
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
    importMedia: props.initialSettings?.importMedia ?? false,
  };
  let initialQuery = undefined;
  if (initialValues.url) {
    initialQuery = { url: initialValues.url };
  }

  const [values, setValues] = createSignal<FormValues>(initialValues);
  const [query, setQuery] = createSignal<FetchUrlJob | undefined>(initialQuery);

  const [queueItems, setQueueItems] = createSignal<QueueItem[]>([]);
  const [importedItems, setImportedItems] = createSignal<BaseEntity[]>([]);

  const onValueChange = throttle((values: FormValues) => {
    const url = values.url;
    if (url) {
      setQuery({ url });
    }
  }, 500);

  let nextItemId = 0;

  const [results] = createResource<LoadState<FetchUrlOutput>, FetchUrlJob>(
    query,
    async (query): Promise<LoadState<FetchUrlOutput>> => {
      if (query && query.url) {
        try {
          const out = await api.fetchUrl(query);
          const state: LoadState<FetchUrlOutput> = {
            state: "success",
            data: out,
          };

          // Find existing.

          const urls: string[] = out.items
            .map((item) => item.data[SEMANTIC_URL])
            .filter((x) => !!x);
          const existingEntities = await api.select({
            ...newSelect(),
            filter: exprIn(
              exprAttr(SEMANTIC_URL),
              exprList(urls.map(exprLiteral))
            ),
            limit: Math.max(urls.length * 2, 500) as any,
          });

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
          setQueueItems(items);

          props.onSettingsChanged?.({
            url: query.url,
            importMedia: values().importMedia,
          });

          return state;
        } catch (error: any) {
          return { state: "error", error: error.toString() };
        }
      } else {
        return { state: "idle" };
      }
    },
    { initialValue: { state: "idle" } }
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

  const form = createForm<FormValues>({
    initialValues: initialValues,
    onValid: onValueChange,
  });

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
            icon="search"
            field={form.field("url")}
            label={null}
            help="Url to import."
            placeholder="https://..."
          />
          <CheckboxField
            field={form.field("importMedia")}
            label={null}
            checkboxLabel={"Import associated media"}
          />
        </form>
      </Box>

      <div>
        <Switch>
          <Match when={results().state === "idle"}>
            <Notification>Enter a url...</Notification>
          </Match>
          <Match when={results().state === "loading"}>{SPINNER}</Match>
          <Match when={loadAsError(results())}>
            {(err) => <NotificationError>{err}</NotificationError>}
          </Match>
          <Match when={loadAsSuccess(results())}>
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
                    let content;
                    if (!!contentRender) {
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

        <Show when={importedItems().length > 0}>
          <h4 class="title is-4">Imported</h4>

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
