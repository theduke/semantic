import { ValueMap } from "semantic/dist/api";
import {
  FACTOR_ID,
  SemanticTag,
  SEMANTIC_TAGS,
  SEMANTIC_TAG_NAME,
} from "semantic/dist/schema";
import { Box, DeletableTag, Tag, Tags } from "solid-bulma";
import { createEffect, createSignal, For, JSX, Show } from "solid-js";
import { buildTagSelect } from ".";
import { Modal } from "../bulma/modal";
import { EntitiesLoader } from "../entity/EntitiesLoader";
import { MultiSelectSearch } from "../util/MultiSelectSearch";
import { PatchOp } from "semantic/dist/core";
import { useApi } from "../../context";
import {
  createLoader,
  loadAsError,
  renderError,
  runWithLoader,
} from "../util/load";
import { isEqual } from "lodash";
import { Button, Buttons } from "../bulma/button";
import { Portal } from "solid-js/web";

export interface EntityTagManagerProps {
  entity: ValueMap;
  modal?: boolean;
  onFinished?: (newTags: string[] | null) => void;
}

export function EntityTagManager(props: EntityTagManagerProps): JSX.Element {
  // TODO: cache tags globally?
  return (
    <EntitiesLoader select={buildTagSelect()}>
      {(tagsRaw) => {
        const api = useApi();
        // TODO: validation
        const allTags = tagsRaw as SemanticTag[];

        const [persistState, setPersistState] = createLoader<void>();

        const findTag = (rawTerm: string): SemanticTag[] => {
          const term = rawTerm.trim().toLowerCase();
          const matches = allTags.filter((tag) =>
            tag[SEMANTIC_TAG_NAME].toLowerCase().includes(term)
          );
          return matches;
        };

        const initialTagIds = (props.entity[SEMANTIC_TAGS] ?? []) as string[];
        const initialTags: SemanticTag[] =
          (initialTagIds
            .map((id) => {
              return allTags.find((tag) => tag[FACTOR_ID] === id);
            })
            .filter((x) => !!x) as SemanticTag[]) ?? [];

        const initialSelection = allTags.filter(
          (tag) => !initialTagIds.includes(tag[FACTOR_ID])
        );

        initialTagIds.sort();

        const [selected, setSelected] =
          createSignal<SemanticTag[]>(initialTags);
        const [isChanged, setIsChanged] = createSignal(false);
        createEffect(() => {
          const selectedTagIds = selected().map((tag) => tag[FACTOR_ID]);
          selectedTagIds.sort();
          const hasChanges = !isEqual(selectedTagIds, initialTagIds);
          setIsChanged(hasChanges);
        });

        const doPersist = async (isFinished: boolean): Promise<string[]> => {
          const oldTags = props.entity[SEMANTIC_TAGS] ?? ([] as string[]);
          const newTags = selected().map((t) => t[FACTOR_ID]);

          const ops: PatchOp[] = [];

          for (const tagId of newTags) {
            if (!oldTags.includes(tagId)) {
              ops.push({
                Add: { path: [{ Key: SEMANTIC_TAGS }], value: tagId },
              });
            }
          }
          for (const tagId of oldTags) {
            if (!newTags.includes(tagId)) {
              ops.push({
                Remove: { path: [{ Key: SEMANTIC_TAGS }], value: tagId },
              });
            }
          }

          if (ops.length > 0) {
            try {
              const out = await runWithLoader(
                setPersistState,
                api.mutate({
                  Patch: { id: props.entity[FACTOR_ID], patch: ops },
                })
              );

              if (isFinished && props.onFinished) {
                props.onFinished(newTags);
              }
            } catch (err: any) {}
          }

          return newTags;
        };

        const onClose = async () => {
          if (persistState().state === "error") {
            props.onFinished?.(null);
            return;
          }
          doPersist(true);
        };

        const select = (
          <MultiSelectSearch<SemanticTag>
            searchSync={findTag}
            defaultItems={initialSelection}
            initialSelection={initialTags}
            onChange={setSelected}
            renderSelected={(items, remove) => {
              const out = (
                <div class="mr-2 ml-2 pt-2">
                  <Tags>
                    <Show
                      when={items().length > 0}
                      fallback={<p class="m-2">No tags selected.</p>}
                    >
                      <For each={items()}>
                        {(tag, index) => {
                          return (
                            <DeletableTag
                              onDelete={() => {
                                remove(index());
                              }}
                            >
                              {tag[SEMANTIC_TAG_NAME]}
                            </DeletableTag>
                          );
                        }}
                      </For>
                    </Show>
                  </Tags>
                </div>
              );

              return out;
            }}
            renderItemWrapper={(items: JSX.Element) => {
              return (
                <div class="ml-2 mr-2 mt-2">
                  <p class="mb-2">
                    <b>Add</b>
                  </p>
                  <Buttons>{items}</Buttons>
                </div>
              );
            }}
            renderItem={(tag, onSelect) => (
              <Button size="is-small" outlined onclick={onSelect}>
                {tag[SEMANTIC_TAG_NAME]}
              </Button>
            )}
          />
        );

        const wrapper = (
          <div>
            {select}

            <Buttons>
              <Button
                disabled={!isChanged()}
                loading={persistState().state === "loading"}
                onclick={() => doPersist(true)}
              >
                Save
              </Button>
              <Button onclick={() => props.onFinished?.(null)}>Cancel</Button>
            </Buttons>

            <Show when={loadAsError(persistState())}>{renderError}</Show>
          </div>
        );

        if (props.modal) {
          return (
            <Portal>
              <Modal onClose={onClose}>
                <Box>
                  <h5 class="title is-5">Tags</h5>
                  {wrapper}
                </Box>
              </Modal>
            </Portal>
          );
        } else {
          return wrapper;
        }
      }}
    </EntitiesLoader>
  );
}
