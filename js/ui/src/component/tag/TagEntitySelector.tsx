import { Id } from "semantic/dist/core";
import {
  FACTOR_ID,
  SemanticTag,
  SEMANTIC_TAG_NAME,
} from "semantic/dist/schema";
import { JSX } from "solid-js";
import { buildTagSelect } from ".";
import { EntitiesLoader } from "../entity/EntitiesLoader";
import { MultiSelectToggleableTags } from "../util/MultiSelectSearch";

export interface TagEntitySelectorProps {
  onChange: (tags: SemanticTag[]) => void;
  initialSelection?: Id[];
}

export function TagEntitySelector(props: TagEntitySelectorProps): JSX.Element {
  return (
    <EntitiesLoader select={buildTagSelect()}>
      {(raw) => {
        const tags: SemanticTag[] = raw as any;
        tags.sort((a, b) => {
          const aName = a[SEMANTIC_TAG_NAME];
          const bName = b[SEMANTIC_TAG_NAME];
          if (aName < bName) {
            return -1;
          } else if (aName > bName) {
            return 1;
          } else {
            return 0;
          }
        });

        const search = (
          term: string,
          selected: SemanticTag[]
        ): SemanticTag[] => {
          term = term.trim().toLowerCase();
          if (term === "") {
            return tags;
          }

          return tags.filter((t) => {
            selected.find((t2) => t2 === t) === undefined &&
              t[SEMANTIC_TAG_NAME].search(term) !== -1;
          });
        };

        const initial: SemanticTag[] =
          props.initialSelection?.reduce((items: SemanticTag[], id) => {
            const t = tags.find((t) => t[FACTOR_ID] === id);
            if (t) {
              items.push(t);
            }
            return items;
          }, []) ?? [];

        const defaultOptions = tags;

        return (
          <MultiSelectToggleableTags
            defaultItems={defaultOptions}
            initialSelection={initial}
            buildTitle={(t) => t[SEMANTIC_TAG_NAME] ?? "??"}
            searchSync={search}
            onChange={props.onChange}
          />
        );
      }}
    </EntitiesLoader>
  );
}
