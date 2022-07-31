import {
  Accessor,
  createSignal,
  For,
  JSX,
  Setter,
  Show,
  untrack,
} from "solid-js";
import { DeletableTag } from "solid-bulma";
import zod from "zod";

import { newSelect } from "semantic/dist/api";
import { useRegistry } from "../../../context";
import { Select } from "semantic/dist/core";
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
import { EntityType } from "../../../semantic";
import { SelectOption } from "../../form/Select";

export const validateEntityFilterData = zod.object({
  type: zod.literal("data"),
  searchTerm: zod.optional(zod.string()),
  entityTypes: zod.optional(zod.array(zod.string())),
});

export type EntityFilterData = zod.infer<typeof validateEntityFilterData>;

export function newFilterData(): EntityFilterData {
  return {
    type: "data",
    searchTerm: "",
  };
}

export function buildFilterDataSelect(filter: EntityFilterData): Select {
  let exprs = [];

  const term = filter.searchTerm?.trim();
  if (term) {
    const contains = exprContains(exprAttr(SEMANTIC_TITLE), exprLiteral(term));
    exprs.push(contains);
  }

  if (filter.entityTypes && filter.entityTypes.length > 0) {
    const types = filter.entityTypes;
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

interface EntityTypeOption extends SelectOption<EntityType> {
  title: string;
}

export function EntityFilterForm(props: EntityFilterFormProps): JSX.Element {
  const [editingType, setEditingType] = createSignal<boolean>(false);
  const reg = useRegistry();

  const entityTypes: EntityTypeOption[] = Object.values(reg.entityTypes).map(
    (type) => ({
      value: type[FACTOR_IDENT],
      label: type[FACTOR_TITLE] ?? type[FACTOR_IDENT],
      title: type[FACTOR_TITLE] ?? type[FACTOR_IDENT],
    })
  );
  const entityTypeLookup: Record<EntityType, EntityTypeOption> = {};
  for (const item of entityTypes) {
    entityTypeLookup[item.value] = item;
  }

  const searchType = (
    term: string,
    afterItem?: EntityTypeOption
  ): Promise<EntityTypeOption[]> => {
    const lower = term.toLowerCase();
    const filtered = entityTypes.filter((t) => {
      return (
        t.title.toLowerCase().search(lower) !== -1 ||
        t.value?.toLowerCase().search(lower) !== -1
      );
    });
    return Promise.resolve(filtered);
  };

  const initialFilter = untrack(() => props.filter());

  return (
    <div>
      <div class="mb-2">
        <SearchInput
          value={initialFilter.searchTerm}
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
                {(type, index) => {
                  const schema = reg.entityTypes[type];

                  return (
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
                      {schema[FACTOR_TITLE] || schema[FACTOR_IDENT]}
                    </DeletableTag>
                  );
                }}
              </For>
            </div>

            <div class="tags"></div>
          </div>

          <Show when={editingType()}>
            <MultiSelectSearch<EntityTypeOption>
              renderSelected={(_selected) => null}
              search={searchType}
              defaultItems={entityTypes.filter(
                (t) => !props.filter()?.entityTypes?.includes(t.value)
              )}
              initialSelection={props
                .filter()
                ?.entityTypes?.flatMap((t) => [entityTypeLookup[t]])}
              onChange={(items) => {
                props.setFilter((old) => ({
                  ...old,
                  entityTypes: items.map((i) => i.value),
                }));
              }}
              renderItemWrapper={(items) => <div class="tags m-2">{items}</div>}
              renderItem={(item, onSelect) => {
                return (
                  <span
                    class="tag is-clickable"
                    title={item.title}
                    onClick={onSelect}
                  >
                    {item.title}
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
