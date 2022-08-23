import {
  FACTOR_ID,
  SemanticTag,
  SEMANTIC_TAG_NAME,
} from "semantic/dist/schema";
import { createSignal, JSX, Show } from "solid-js";
import { exprSearchTagByName } from ".";
import { useApi, useRegistry } from "../../context";
import { Button, Buttons } from "../bulma/button";
import { EntityPicker } from "../entity/EntityPicker";
import {
  createLoader,
  loadAsError,
  renderError,
  startLoader,
} from "../util/load";

export interface TagMergerProps {
  tag: SemanticTag;
  onCancel: () => void;
  onMerged: (mergedTag: SemanticTag) => void;
}

export function TagMerger(props: TagMergerProps): JSX.Element {
  const reg = useRegistry();
  const api = useApi();

  const title = props.tag[SEMANTIC_TAG_NAME] || reg.entityTitle(props.tag);

  const [targetTag, setTargetTag] = createSignal<SemanticTag | undefined>();
  const [loader, setLoader] = createLoader<void>();

  const runMerge = (target: SemanticTag) => {
    startLoader([loader, setLoader], () =>
      api.tagMerge(props.tag[FACTOR_ID], target[FACTOR_ID]).then(() => {
        props.onMerged(target);
      })
    );
  };
  return (
    <div>
      <h5 class="title is-5">Merge Tag {title}</h5>

      <Show
        when={targetTag()}
        fallback={
          <EntityPicker
            buildFilter={exprSearchTagByName}
            renderItemLabel={(entity) =>
              entity[SEMANTIC_TAG_NAME] ?? reg.entityTitle(entity)
            }
            onSelect={setTargetTag}
          />
        }
      >
        {(selectedTag) => {
          return (
            <div>
              <Show when={loadAsError(loader())}>{renderError}</Show>

              <Button
                loading={loader().state === "loading"}
                onclick={() => runMerge(selectedTag)}
              >
                Merge {title} into{" "}
                {selectedTag[SEMANTIC_TAG_NAME] || reg.entityTitle(selectedTag)}
              </Button>
            </div>
          );
        }}
      </Show>

      <Buttons class="mt-4">
        <Button onclick={props.onCancel}>Cancel</Button>
      </Buttons>
    </div>
  );
}
