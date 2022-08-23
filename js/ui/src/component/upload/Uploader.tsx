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
  createEffect,
  createSignal,
  For,
  JSX,
  onCleanup,
  onMount,
  Show,
  Signal,
} from "solid-js";
import { useApi, useRegistry } from "../../context";
import { Button, Buttons, IconButton } from "../bulma/button";
import { FieldHorizontal } from "../bulma/form";
import { Notification, NotificationError } from "../bulma/notification";
import { StatefulEntityPicker } from "../entity/StatefulEntityPicker";
import { prettyPrintByteSize } from "../util";
import {
  LoadState,
  createLoader,
  startLoader,
  loadAsError,
  SPINNER,
} from "../util/load";

export interface UploaderProps {}

type QueueId = number;

interface QueueItem {
  id: QueueId;
  file: File;
  loader: Signal<LoadState<FileUploadReply>>;
}

export function Uploader(_props: UploaderProps): JSX.Element {
  const reg = useRegistry();
  const api = useApi();

  let queueId = 0;
  let metadata: MetaFormValues = {};

  const [queue, setQueue] = createSignal<QueueItem[]>([]);
  const [uploaded, setUploaded] = createSignal<FileUploadReply[]>([]);
  const [metaFormVisible, setMetaFormVisible] = createSignal(false);

  const addFiles = (files: FileList) => {
    const newFiles = Array.from(files).map(
      (file): QueueItem => ({ id: queueId++, file, loader: createLoader() })
    );
    setQueue((queue) => [...queue, ...newFiles]);
  };

  const onInputChange = (files: FileList | null) => {
    if (files) {
      addFiles(files);
    }
  };

  const removeQueueItem = (item: QueueItem) => {
    setQueue((queue) => queue.filter((i) => i.id !== item.id));
  };

  const [status, setStatus] = createSignal<"idle" | "uploading" | "aborted">(
    "idle"
  );
  onCleanup(() => {
    setStatus("aborted");
  });

  const pasteHandler = (e: ClipboardEvent) => {
    const files = e.clipboardData?.files;
    if (files) {
      addFiles(files);
    }
  };
  onMount(() => {
    document.addEventListener("paste", pasteHandler);
  });
  onCleanup(() => {
    document.removeEventListener("paste", pasteHandler);
  });

  const doUpload = (
    item: QueueItem
  ): Promise<LoadState<FileUploadReply>> | null => {
    console.log("uploading item", { item });

    if (item.loader[0]().state === "loading") {
      return null;
    }

    setStatus("uploading");
    return startLoader(item.loader, async () => {
      const meta: FileUploadMetadata = {
        filename: item.file.name,
        title: null,
        ident: null,
        url: null,
        parent: metadata?.parent?.[FACTOR_ID] ?? null,
        collection_id: metadata?.collection?.[FACTOR_ID] ?? null,
        tag_ids: [],
      };
      console.log("starting upload", { item, meta });
      const res = await api.uploadFile(item.file, meta);
      console.log("upload finished", { item, meta, res });

      if (status() !== "aborted") {
        setQueue((queue) => queue.filter((i) => i.id !== item.id));
        setUploaded((uploaded) => [...uploaded, res]);
        setStatus("idle");
      }
      return res;
    });
  };

  const uploadAll = async () => {
    while (status() !== "aborted") {
      const item = queue().find((i) => i.loader[0]().state === "idle");
      if (!item) {
        setStatus("idle");
        break;
      }
      await doUpload(item);
    }
  };

  const reset = () => {
    setQueue([]);
    setUploaded([]);
    metadata = {};
    setMetaFormVisible(false);
    // TODO: clear metadata
  };

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
            disabled={metaFormVisible()}
            onclick={() => setMetaFormVisible(true)}
          >
            Add Metadata
          </IconButton>

          <Button
            size="is-medium"
            onClick={reset}
            disabled={
              status() === "idle" &&
              queue().length < 1 &&
              uploaded().length < 1 &&
              !metaFormVisible()
            }
          >
            Reset
          </Button>
        </Buttons>
      </Box>

      <Show when={metaFormVisible()}>
        <div class="mt-4 mb-4">
          <UploaderMetaForm
            onChange={(data) => {
              metadata = data;
            }}
            onCancel={() => setMetaFormVisible(false)}
          />
        </div>
      </Show>

      <Show when={queue().length > 0}>
        <h4 class="title is-4">Queue</h4>

        <Buttons>
          <Show
            when={status() === "idle"}
            fallback={() => {
              return (
                <div>
                  {SPINNER}
                  <span>Uploading {queue().length} files...</span>
                </div>
              );
            }}
          >
            <IconButton icon="upload" size="is-medium" onclick={uploadAll}>
              Upload All ({queue().length})
            </IconButton>

            <IconButton
              icon="trash"
              size="is-medium"
              onclick={() => setQueue([])}
            >
              Clear
            </IconButton>
          </Show>
        </Buttons>
        <For
          each={queue()}
          fallback={<Notification>Select files above...</Notification>}
        >
          {(item: QueueItem) => (
            <QueueItem
              item={item}
              onRemove={removeQueueItem}
              startUpload={doUpload}
            />
          )}
        </For>
      </Show>

      <Show when={uploaded().length > 0}>
        <h4 class="title is-4">Uploaded</h4>
        <Buttons>
          <Button
            size="is-medium"
            disabled={uploaded().length < 1}
            onclick={() => {
              setUploaded([]);
            }}
          >
            Clear
          </Button>
        </Buttons>

        <div
          style={{ display: "flex", "flex-direction": "column", gap: "1rem" }}
        >
          <For
            each={uploaded()}
            fallback={<Notification>No files uploaded yet...</Notification>}
          >
            {(item) => {
              return reg.renderEntity(item.file, { preview: true });
            }}
          </For>
        </div>
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
  const { item, onRemove, startUpload } = props;

  const [loading, setLoading] = createSignal(
    item.loader[0]().state === "loading"
  );

  createEffect(() => {
    const sig = item.loader[0]();
    setLoading(sig.state === "loading");
  });

  return (
    <Box>
      <div>
        <div class="columns">
          <div class="column is-flex-grow-1">
            <div class="mb-2">
              <b>{item.file.name}</b>
            </div>
            <div>
              <span>{item.file.type}</span>
              <span class="pl-2">{prettyPrintByteSize(item.file.size)}</span>
            </div>
          </div>
          <div class="column">
            <Show
              when={loading()}
              fallback={
                <Buttons class="is-justify-content-flex-end">
                  <IconButton
                    icon="upload"
                    loading={loading()}
                    onClick={() => startUpload(item)}
                  >
                    Upload
                  </IconButton>
                  <IconButton
                    icon="trash"
                    onclick={() => onRemove(item)}
                    disabled={loading()}
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
        <Show when={loadAsError(item.loader[0]())}>
          {(error) => (
            <div class="mt-3">
              <NotificationError>{error}</NotificationError>
            </div>
          )}
        </Show>
      </div>
    </Box>
  );
}
