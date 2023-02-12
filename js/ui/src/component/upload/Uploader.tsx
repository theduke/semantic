import { api } from "semantic";
import { FileUploadMetadata, FileUploadReply } from "semantic/dist/core";
import { exprIsEntityType } from "semantic/dist/db";
import {
  BaseEntity,
  FACTOR_ID,
  FACTOR_IDENT,
  SemanticCollection,
  SemanticTag,
  TY_SEMANTIC_COLLECTION,
} from "semantic/dist/schema";
import { Box, FileInput } from "solid-bulma";
import {
  Accessor,
  createResource,
  createSignal,
  ErrorBoundary,
  For,
  JSX,
  Match,
  onCleanup,
  onMount,
  Resource,
  ResourceActions,
  Setter,
  Show,
  Signal,
  Switch,
} from "solid-js";
import { useApi, useRegistry } from "../../context";
import { Button, Buttons, IconButton } from "../bulma/button";
import { FieldHorizontal } from "../bulma/form";
import { Notification, NotificationError } from "../bulma/notification";
import { StatefulEntityPicker } from "../entity/StatefulEntityPicker";
import { prettyPrintByteSize } from "../util";
import { SPINNER } from "../util/load";

export interface UploaderProps {}

type QueueId = number;

interface QueueItem {
  id: QueueId;
  file: File;
  uploading: Signal<boolean>;
  resource: Resource<FileUploadReply>;
  actions: ResourceActions<FileUploadReply | undefined>;
}

type Status = "idle" | "uploadingItem" | "uploadingAll" | "aborted";

class State {
  private api: api.Api;

  status: Accessor<Status>;
  private setStatus: Setter<Status>;

  queue: Accessor<QueueItem[]>;
  private setQueue: Setter<QueueItem[]>;

  metaFormVisible: Accessor<boolean>;
  setMetaFormVisible: Setter<boolean>;

  metadata: MetaFormValues;
  private nextQueueId: number;

  constructor(api: api.Api) {
    this.api = api;

    const [status, setStatus] = createSignal<Status>("idle");
    this.status = status;
    this.setStatus = setStatus;

    const [queue, setQueue] = createSignal<QueueItem[]>([]);
    this.queue = queue;
    this.setQueue = setQueue;

    const [metaFormVisible, setMetaFormVisible] = createSignal<boolean>(false);
    this.metaFormVisible = metaFormVisible;
    this.setMetaFormVisible = setMetaFormVisible;

    this.metadata = {};
    this.nextQueueId = 0;
  }

  toggleMetaFormVisible() {
    this.setMetaFormVisible((v) => !v);
  }

  addFiles(files: FileList) {
    const newFiles = Array.from(files).map((file): QueueItem => {
      const uploading = createSignal<boolean>(false);

      const id = this.nextQueueId++;

      const [resource, actions] = createResource<
        FileUploadReply,
        Accessor<boolean>
      >(uploading[0], async (_) => {
        const meta: FileUploadMetadata = {
          filename: file.name,
          title: null,
          ident: null,
          url: null,
          parent: this.metadata.parent?.[FACTOR_ID] ?? null,
          collection_id: this.metadata.collection?.[FACTOR_ID] ?? null,
          tag_ids: [],
        };
        console.log("starting upload", { id, file, meta });
        try {
          const res = await this.api.uploadFile(file, meta);
          console.log("upload finished", { id, file, meta, res });

          if (this.status() === "uploadingAll") {
            this.uploadAll();
          } else {
            this.setStatus("idle");
          }

          this.setQueue((q) => [...q]);

          return res;
        } catch (err) {
          console.error("upload failed", { id, file, meta, err });

          this.setStatus("idle");

          throw err;
        }
      });

      return { id, file, uploading, resource, actions };
    });
    this.setQueue((queue) => [...queue, ...newFiles]);
  }

  removeItem(item: QueueItem) {
    this.setQueue((queue) => queue.filter((i) => i.id !== item.id));
  }

  abort() {
    this.setStatus("aborted");
  }

  startItemUpload(item: QueueItem, all: boolean) {
    if (!item.uploading[0]()) {
      if (item.resource.error) {
        item.actions.refetch();
      } else {
        item.uploading[1](true);
      }
    }
    if (!all) {
      this.setStatus("uploadingItem");
    }
  }

  async uploadAll() {
    while (this.status() !== "aborted") {
      const item = this.queue().find(
        (i) => !i.resource.loading && i.resource.latest
      );
      if (!item) {
        this.setStatus("idle");
        break;
      }
      this.setStatus("uploadingAll");
      this.startItemUpload(item, true);
    }
  }

  reset() {
    this.setQueue([]);
    this.metadata = {};
    this.setMetaFormVisible(false);
  }

  hasUploaded() {
    return this.queue().some((i) => i.resource.state === "ready");
  }

  clearQueue() {
    this.setQueue([]);
  }

  uploadedReplies(): FileUploadReply[] {
    const replies = [];
    for (const item of this.queue()) {
      if (item.resource.state === "ready" && item.resource.latest) {
        replies.push(item.resource.latest);
      }
    }
    return replies;
  }
}

export function Uploader(_props: UploaderProps): JSX.Element {
  const reg = useRegistry();
  const state = new State(useApi());

  const onInputChange = (files: FileList | null) => {
    if (files) {
      state.addFiles(files);
    }
  };

  onCleanup(() => {
    state.abort();
  });

  const pasteHandler = (e: ClipboardEvent) => {
    const files = e.clipboardData?.files;
    if (files) {
      state.addFiles(files);
    }
  };
  onMount(() => {
    document.addEventListener("paste", pasteHandler);
  });
  onCleanup(() => {
    document.removeEventListener("paste", pasteHandler);
  });

  return (
    <div>
      <Box>
        <div class="columns">
          <div class="column">
            <FileInput
              boxed
              onFilesChange={onInputChange}
              label="Choose files..."
              multiple
            />
          </div>
          <div class="column">
            <Notification>Drag files into the box.</Notification>
          </div>
          <div class="column">
            <Notification>
              Use CTRL+V to paste files from the clipboard.
            </Notification>
          </div>
        </div>

        <Buttons class="mt-2">
          <IconButton
            icon="pencil"
            size="is-medium"
            disabled={state.metaFormVisible()}
            onclick={() => state.toggleMetaFormVisible()}
          >
            Add Metadata
          </IconButton>

          <Button
            size="is-medium"
            onClick={() => state.reset()}
            disabled={
              state.status() === "idle" &&
              state.queue().length < 1 &&
              !state.metaFormVisible()
            }
          >
            Reset
          </Button>
        </Buttons>
      </Box>

      <Show when={state.metaFormVisible()}>
        <div class="mt-4 mb-4">
          <UploaderMetaForm
            onChange={(data) => {
              state.metadata = data;
            }}
            onCancel={() => state.setMetaFormVisible(false)}
          />
        </div>
      </Show>

      <Show when={state.queue().length > 0}>
        <h4 class="title is-4">Queue</h4>

        <Buttons>
          <Show
            when={state.queue().find((x) => {
              const res = x.resource;
              return res.loading || !res.latest;
            })}
          >
            <IconButton
              loading={state.status() === "uploadingAll"}
              icon="upload"
              size="is-medium"
              onclick={() => state.uploadAll()}
            >
              Upload all ({state.queue().length})
            </IconButton>

            <IconButton
              icon="trash"
              size="is-medium"
              disabled={
                state.status() === "uploadingAll" ||
                state.status() === "uploadingItem"
              }
              onclick={() => state.clearQueue()}
            >
              Clear
            </IconButton>
          </Show>
        </Buttons>
        <For
          each={state.queue()}
          fallback={<Notification>Select files above...</Notification>}
        >
          {(item: QueueItem) => (
            <QueueItem
              item={item}
              onRemove={(x) => state.removeItem(x)}
              startUpload={(x) => state.startItemUpload(x, false)}
            />
          )}
        </For>
      </Show>
    </div>
  );
}

interface MetaFormValues {
  collection?: SemanticCollection;
  parent?: BaseEntity;
  tags?: SemanticTag[];
}

interface UploaderMetaFormProps {
  onChange: (values: MetaFormValues) => void;
  onCancel: () => void;
}

function UploaderMetaForm(props: UploaderMetaFormProps): JSX.Element {
  const reg = useRegistry();
  const collectionSchema = Object.values(reg.classes).find(
    (e) => e[FACTOR_IDENT] === TY_SEMANTIC_COLLECTION
  );
  if (!collectionSchema) {
    throw new Error("Could not find collection schema");
  }

  const values: MetaFormValues = {};

  return (
    <Box>
      <h5 class="title is-5">Metadata</h5>
      <div>
        <FieldHorizontal label={"Collection"}>
          <Box>
            <StatefulEntityPicker
              baseFilter={exprIsEntityType(TY_SEMANTIC_COLLECTION)}
              schema={collectionSchema}
              onChange={(value) => {
                values.collection = value as SemanticCollection | undefined;
                props.onChange(values);
              }}
            />
          </Box>
        </FieldHorizontal>
        <FieldHorizontal label={"Parent"}>
          <Box>
            <StatefulEntityPicker
              noCreate
              schema={collectionSchema}
              onChange={(value) => {
                values.parent = value as BaseEntity | undefined;
                props.onChange(values);
              }}
            />
          </Box>
        </FieldHorizontal>

        <Buttons>
          <Button onClick={props.onCancel}> Clear </Button>
        </Buttons>
      </div>
    </Box>
  );
}

interface QueueItemProps {
  item: QueueItem;
  onRemove: (item: QueueItem) => void;
  startUpload: (item: QueueItem) => void;
}

function QueueItem(props: QueueItemProps): JSX.Element {
  const reg = useRegistry();

  return (
    <Switch>
      <Match when={props.item.resource.latest} keyed>
        {(reply) => reg.renderEntity(reply.file, { preview: true })}
      </Match>

      <Match when={true}>
        <Box>
          <div>
            <div class="columns">
              <div class="column is-flex-grow-1">
                <div class="mb-2">
                  <b>{props.item.file.name}</b>
                </div>
                <div>
                  <span>{props.item.file.type}</span>
                  <span class="pl-2">
                    {prettyPrintByteSize(props.item.file.size)}
                  </span>
                </div>
              </div>
              <div class="column">
                <Show
                  when={props.item.resource.loading}
                  fallback={
                    <Buttons class="is-justify-content-flex-end">
                      <IconButton
                        icon="upload"
                        onClick={() => props.startUpload(props.item)}
                      >
                        Upload
                      </IconButton>
                      <IconButton
                        icon="trash"
                        onclick={() => props.onRemove(props.item)}
                      >
                        Remove
                      </IconButton>
                    </Buttons>
                  }
                >
                  {/* TODO: show progress */}
                  <Button loading={true} />
                </Show>
              </div>
            </div>
            <Show when={props.item.resource.error} keyed>
              {(error: any) => (
                <div class="mt-3">
                  <NotificationError>{error.toString()}</NotificationError>
                </div>
              )}
            </Show>
          </div>
        </Box>
      </Match>
    </Switch>
  );
}
