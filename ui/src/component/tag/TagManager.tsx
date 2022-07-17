import { createSignal, For, Show } from "solid-js";
import { JSX } from "solid-js/jsx-runtime";
import { newSelect } from "../../api";
import { Item, Page, Select } from "../../semantic/core";
import { exprAttr, exprIsEntityType } from "../../semantic/db";
import {
  SemanticTag,
  SEMANTIC_TITLE,
  TY_SEMANTIC_TAG,
} from "../../semantic/schema";
import { Button, Buttons } from "../bulma/button";
import { NotificationWarning } from "../bulma/notification";
import { EntitiesLoader } from "../entity/EntitiesLoader";
import { EntityDeleterModal } from "../entity/EntityDeleterModal";
import { GenericPage } from "../util";
import { TagCreate } from "./TagCreate";

export function TagManagerPage(): JSX.Element {
  const select: Select = {
    ...newSelect(),
    // FIXME: no bigint (needs type change)
    limit: 20_000 as any,
    filter: exprIsEntityType(TY_SEMANTIC_TAG),
    sort: [{ on: exprAttr(SEMANTIC_TITLE), order: "Asc" }],
  };

  return (
    <GenericPage title="Tags">
      <EntitiesLoader select={select}>
        {(page) => {
          // TODO: type checks with yup.
          const typedPage = page as Page<Item<SemanticTag>>;
          return <TagManager page={typedPage} />;
        }}
      </EntitiesLoader>
    </GenericPage>
  );
}

export interface TagManagerProps {
  page: Page<Item<SemanticTag>>;
}

function TagManager(props: TagManagerProps): JSX.Element {
  const sortedTags = props.page.items
    .map((x) => x.data)
    .sort((a, b) => {
      return a === b ? 0 : a < b ? -1 : 1;
    });

  const [tags, setTags] = createSignal(sortedTags);
  const [tagDelete, setTagDelete] = createSignal<SemanticTag | null>(null);
  const [creating, setCreating] = createSignal<boolean>(false);

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

  return (
    <div>
      <Show when={tagDelete()}>
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
      </Show>

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
                <Button>{tag["semantic/tag_name"]}</Button>
                <Button
                  size="is-small"
                  color="is-danger"
                  onClick={[startTagDelete, tag]}
                >
                  Delete
                </Button>
              </Buttons>
            </div>
          )}
        </For>
      </div>
    </div>
  );
}
