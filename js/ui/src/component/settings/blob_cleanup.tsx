import { createResource, For, JSX, Show } from "solid-js";
import { useApi } from "../../context";
import {
  FallibleResourceLoader,
  createFallibleResource,
  FallibleResource,
} from "../util/load";

import { Notification } from "../bulma/notification";
import { GenericPage, prettyPrintByteSize } from "../util";
import { Button, Buttons } from "../bulma/button";
import { BlobInfo } from "semantic/dist/core";

function UnusedBlobLoader(props: {
  children: (info: BlobInfo[]) => JSX.Element;
}): JSX.Element {
  const api = useApi();
  const load = () => api.findUnusedBlobs();
  return <FallibleResourceLoader load={load} children={props.children} />;
}

export function BlobCleaner(): JSX.Element {
  const api = useApi();

  return (
    <FallibleResourceLoader load={() => api.findUnusedBlobs()}>
      {(items, actions) => {
        const totalSize = items.reduce(
          (acc, item) => acc + (item.size as any as number),
          0
        );
        let exec: FallibleResource<any> | undefined;

        const doDelete = () => {
          if (exec?.loading) {
            return;
          }
          const [rr] = createFallibleResource(async () => {
            await api.deleteUnusedBlobs();
            actions.refetch();
            return true;
          });
          exec = rr;
        };

        return (
          <div>
            <Show
              when={items.length > 0}
              fallback={() => (
                <Notification color="is-success">
                  No unused blobs found.
                </Notification>
              )}
            >
              <Notification color="is-info">
                Found <b>{items.length}</b> unused blob(s).{" "}
                <b>{prettyPrintByteSize(totalSize)}</b> can be freed.
              </Notification>

              <Buttons>
                <Button
                  outlined
                  color="is-danger"
                  loading={exec?.loading}
                  size="is-medium"
                  class="mb-4"
                  onclick={doDelete}
                >
                  Delete all unused blobs
                </Button>
              </Buttons>

              <table class="table">
                <thead>
                  <tr>
                    <th>Path</th>
                    <th>Size</th>
                  </tr>
                </thead>

                <For each={items}>
                  {(item) => {
                    return (
                      <tr>
                        <td>{item.key}</td>
                        <td>
                          {prettyPrintByteSize(item.size as any as number)}
                        </td>
                      </tr>
                    );
                  }}
                </For>
              </table>
            </Show>
          </div>
        );
      }}
    </FallibleResourceLoader>
  );
}

export function BlobCleanupPage(): JSX.Element {
  return (
    <GenericPage title="Blob Cleanup">
      <BlobCleaner />
    </GenericPage>
  );
}
