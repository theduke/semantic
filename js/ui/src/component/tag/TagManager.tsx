import { createSignal, For, Match, Show, Switch } from "solid-js";
import { JSX } from "solid-js/jsx-runtime";
import { Select } from "semantic/dist/core";
import { SemanticTag, SEMANTIC_TAG_NAME } from "semantic/dist/schema";
import { Button, Buttons } from "../bulma/button";
import { NotificationWarning } from "../bulma/notification";
import { EntitiesLoader } from "../entity/EntitiesLoader";
import { EntityDeleterModal } from "../entity/EntityDeleterModal";
import { GenericPage } from "../util";
import { TagCreate } from "./TagCreate";
import { Link } from "solid-app-router";
import { entityLinkPath } from "../../semantic";
import { Modal } from "../bulma/modal";
import { TagMerger } from "./TagMerger";
import { Box } from "solid-bulma";
import { buildTagSelect } from ".";

export function TagManagerPage(): JSX.Element {
  const select: Select = buildTagSelect();

  return (
    <GenericPage title="Tags">
      <EntitiesLoader select={select}>
        {(page) => {
          // TODO: type checks with yup.
          const typedPage = page as SemanticTag[];
          return <TagManager items={typedPage} />;
        }}
      </EntitiesLoader>
    </GenericPage>
  );
}

export interface TagManagerProps {
  items: SemanticTag[];
}

function TagManager(props: TagManagerProps): JSX.Element {
  const sortedTags = props.items.sort((a, b) => {
    const an = a["semantic/tag_name"];
    const bn = b["semantic/tag_name"];
    return an === bn ? 0 : an < bn ? -1 : 1;
  });

  const [tags, setTags] = createSignal(sortedTags);
  const [tagDelete, setTagDelete] = createSignal<SemanticTag | null>(null);
  const [creating, setCreating] = createSignal<boolean>(false);
  const [merging, setMerging] = createSignal<SemanticTag | null>(null);

  const onTagMerged = (sourceTag: SemanticTag, _newTag: SemanticTag) => {
    setTags((tags) => tags.filter((tag) => tag !== sourceTag));
    setMerging(null);
  };

  const addTag = (tag: SemanticTag) => {
    const name = tag["semantic/tag_name"];

    const nextIndex = tags().findIndex((t) => t["semantic/tag_name"] >= name);

    if (nextIndex === 0) {
      setTags((tags) => [tag, ...tags]);
    } else if (nextIndex > 0) {
      setTags((tags) => [
        ...tags.slice(0, nextIndex),
        tag,
        ...tags.slice(nextIndex),
      ]);
    } else {
      setTags((tags) => [...tags, tag]);
    }
  };

  const onTagDeleted = (tag: SemanticTag) => {
    const id = tag["factor/id"];
    setTags((tags) => {
      const index = tags.findIndex((t) => t["factor/id"] === id);
      if (typeof index === "number") {
        return [...tags.slice(0, index), ...tags.slice(index + 1)];
      } else {
        return tags;
      }
    });
    setTagDelete(null);
  };
  const onDeleteCancel = () => {
    setTagDelete(null);
  };
  const startTagDelete = (tag: SemanticTag) => {
    setTagDelete(tag);
  };
  const startTagMerge = (tag: SemanticTag) => {
    setMerging(tag);
  };

  return (
    <div>
      <Switch>
        <Match when={tagDelete()}>
          {(tag) => {
            return (
              <EntityDeleterModal
                entity={tag}
                confirmationContent={
                  <p>Really delete tag {tag["semantic/tag_name"]}?</p>
                }
                onDeleted={onTagDeleted}
                onCancel={onDeleteCancel}
              />
            );
          }}
        </Match>

        <Match when={merging()}>
          {(sourceTag) => (
            <Modal onClose={() => setMerging(null)}>
              <Box>
                <TagMerger
                  tag={sourceTag}
                  onCancel={() => setMerging(null)}
                  onMerged={(newTag: SemanticTag) =>
                    onTagMerged(sourceTag, newTag)
                  }
                />
              </Box>
            </Modal>
          )}
        </Match>
      </Switch>

      <Show
        when={creating()}
        fallback={
          <div class="mb-3 mt-3">
            <Button color="is-info" onclick={() => setCreating(true)}>
              New Tag
            </Button>
          </div>
        }
      >
        <div class="box">
          <TagCreate onCreated={addTag} onCancel={() => setCreating(false)} />
        </div>
      </Show>

      <hr />

      <div class="is-flex is-flex-direction-column" style={{ gap: "1rem" }}>
        <For
          each={tags()}
          fallback={<NotificationWarning>No tags found.</NotificationWarning>}
        >
          {(tag) => (
            <div>
              <Buttons>
                <Link class="button" href={entityLinkPath(tag)}>
                  {tag[SEMANTIC_TAG_NAME]}
                </Link>
                <Button
                  size="is-small"
                  color="is-danger"
                  onClick={[startTagDelete, tag]}
                >
                  Delete
                </Button>
                <Button size="is-small" onClick={[startTagMerge, tag]}>
                  Merge
                </Button>
              </Buttons>
            </div>
          )}
        </For>
      </div>
    </div>
  );
}
