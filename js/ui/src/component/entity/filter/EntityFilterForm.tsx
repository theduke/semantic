import { Accessor, createSignal, For, JSX, Setter, Show } from "solid-js";
import { DeletableTag } from "solid-bulma";

import { newSelect } from "semantic/dist/api";
import { useRegistry } from "../../../context";
import { EntitySchema, Select } from "semantic/dist/core";
import {
  exprAndMany,
  exprAttr,
  exprContains,
  exprIsInEntityTypes,
  exprLiteral,
} from "semantic/dist/db";
import {
  FACTOR_IDENT,
  FACTOR_TITLE,
  SEMANTIC_TITLE,
} from "semantic/dist/schema";
import { Button } from "../../bulma/button";
import { FieldHorizontal } from "../../bulma/form";
import { Icon } from "../../bulma/icon";
import { SearchInput } from "../../bulma/SearchInput";
import { MultiSelectSearch } from "../../util/MultiSelectSearch";

export interface EntityFilterData {
  type: "data";
  searchTerm: string;
  entityTypes?: EntitySchema[];
}

export function newFilterData(): EntityFilterData {
  return {
    type: "data",
    searchTerm: "",
  };
}

export function buildFilterDataSelect(filter: EntityFilterData): Select {
  let exprs = [];

  const term = filter.searchTerm.trim();
  if (term) {
    const contains = exprContains(exprAttr(SEMANTIC_TITLE), exprLiteral(term));
    exprs.push(contains);
  }

  if (filter.entityTypes && filter.entityTypes.length > 0) {
    const types = filter.entityTypes.map((schema) => schema[FACTOR_IDENT]);
    exprs.push(exprIsInEntityTypes(types));
  }

  return {
    ...newSelect(),
    filter: exprAndMany(exprs),
  };
}

export interface EntityFilterFormProps {
  filter: Accessor<EntityFilterData>;
  setFilter: Setter<EntityFilterData>;

  autoUpdate?: boolean;
}

export function EntityFilterForm(props: EntityFilterFormProps): JSX.Element {
  const [editingType, setEditingType] = createSignal<boolean>(false);
  const reg = useRegistry();

  const entityTypes = Object.values(reg.entityTypes);

  const searchType = (
    term: string,
    afterItem?: EntitySchema
  ): Promise<EntitySchema[]> => {
    const lower = term.toLowerCase();
    const filtered = entityTypes.filter((t) => {
      return (
        t["factor/title"]?.toLowerCase().search(lower) !== -1 ||
        t["factor/ident"]?.toLowerCase().search(lower) !== -1
      );
    });
    return Promise.resolve(filtered);
  };

  return (
    <div>
      <div class="mb-2">
        <SearchInput
          onInput={(e) =>
            props.setFilter((old) => ({
              ...old,
              searchTerm: e.currentTarget.value,
            }))
          }
        />
      </div>

      <FieldHorizontal label="Type" smallLabel>
        <div class="control">
          <div class="is-flex">
            <div class="mr-4">
              <Button
                onClick={() => {
                  setEditingType((old) => !old);
                }}
              >
                <Icon icon="plus" />
              </Button>
            </div>

            <div>
              <For each={props.filter()?.entityTypes}>
                {(type, index) => (
                  <DeletableTag
                    onDelete={() => {
                      props.setFilter((old) => {
                        const fixed = [...(old?.entityTypes ?? [])];
                        fixed.splice(index(), 1);

                        return {
                          ...old,
                          entityTypes: fixed,
                        };
                      });
                    }}
                  >
                    {type[FACTOR_TITLE] || type[FACTOR_IDENT]}
                  </DeletableTag>
                )}
              </For>
            </div>

            <div class="tags"></div>
          </div>

          <Show when={editingType()}>
            <MultiSelectSearch<EntitySchema>
              renderSelected={(_selected) => null}
              search={searchType}
              defaultItems={entityTypes.filter(
                (t) => !props.filter()?.entityTypes?.includes(t)
              )}
              initialSelection={props.filter()?.entityTypes}
              onChange={(items) => {
                props.setFilter((old) => ({
                  ...old,
                  entityTypes: items,
                }));
              }}
              renderItemWrapper={(items) => <div class="tags">{items}</div>}
              renderItem={(item, onSelect) => {
                const title = item[FACTOR_TITLE] ?? item[FACTOR_IDENT];
                const ty = item[FACTOR_IDENT];
                return (
                  <span class="tag is-clickable" title={ty} onClick={onSelect}>
                    {title}
                  </span>
                );
              }}
            />
          </Show>
        </div>
      </FieldHorizontal>
    </div>
  );
}
