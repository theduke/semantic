import {
  Accessor,
  createSignal,
  For,
  JSX,
  Setter,
  Show,
  untrack,
} from "solid-js";
import { DeletableTag, Tags } from "solid-bulma";
import { useRegistry } from "../../../context";
import { FACTOR_IDENT, FACTOR_TITLE } from "semantic/dist/schema";
import { Button } from "../../bulma/button";
import { FieldHorizontal } from "../../bulma/form";
import { Icon } from "../../bulma/icon";
import { SearchInput } from "../../bulma/SearchInput";
import { MultiSelectSearch } from "../../util/MultiSelectSearch";
import { EntityType } from "../../../semantic";
import { SelectOption } from "../../form/Select";
import { EntityFilterData } from ".";

export interface EntityFilterBuilderFormProps {
  filter: Accessor<EntityFilterData>;
  setFilter: Setter<EntityFilterData>;

  autoUpdate?: boolean;
}

interface EntityTypeOption extends SelectOption<EntityType> {
  title: string;
}

export function EntityFilterBuilderForm(
  props: EntityFilterBuilderFormProps
): JSX.Element {
  const [editingType, setEditingType] = createSignal<boolean>(false);
  const reg = useRegistry();

  const entityTypes: EntityTypeOption[] = Object.values(reg.classes).map(
    (type) => ({
      value: type[FACTOR_IDENT],
      label: type[FACTOR_TITLE] ?? type[FACTOR_IDENT],
      title: type[FACTOR_TITLE] ?? type[FACTOR_IDENT],
    })
  );
  entityTypes.sort((a, b) => {
    if (a.value < b.value) {
      return -1;
    } else if (a.value > b.value) {
      return 1;
    } else {
      return 0;
    }
  });

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

            <Tags>
              <For each={props.filter()?.entityTypes} fallback={<p>All</p>}>
                {(type, index) => {
                  const schema = reg.classes[type];

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
            </Tags>

            <div class="tags"></div>
          </div>

          <Show when={editingType()}>
            {() => {
              console.log("rendering multiselect", props.filter()?.entityTypes);
              return (
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
                  renderItemWrapper={(items) => (
                    <div class="tags m-2">{items}</div>
                  )}
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
              );
            }}
          </Show>
        </div>
      </FieldHorizontal>
    </div>
  );
}
