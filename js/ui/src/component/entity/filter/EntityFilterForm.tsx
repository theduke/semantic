import { Accessor, createSignal, JSX, Setter, untrack } from "solid-js";
import { useRegistry } from "../../../context";
import { FACTOR_ID, FACTOR_IDENT, FACTOR_TITLE } from "semantic/dist/schema";
import { FieldHorizontal } from "../../bulma/form";
import { SearchInput } from "../../bulma/SearchInput";
import { MultiSelectToggleableTags } from "../../util/MultiSelectSearch";
import { EntityType } from "../../../semantic";
import { SelectOption } from "../../form/Select";
import { EntityFilterData } from ".";
import { TagEntitySelector } from "../../tag/TagEntitySelector";
import { SortSelector } from "./SortSelector";

export interface FilterBuilderProps {
  filter: Accessor<EntityFilterData>;
  setFilter: Setter<EntityFilterData>;

  autoUpdate?: boolean;
}

interface EntityTypeOption extends SelectOption<EntityType> {
  title: string;
}

export function FilterBuilder(props: FilterBuilderProps): JSX.Element {
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
    if (a.title < b.title) {
      return -1;
    } else if (a.title > b.title) {
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
    selected: EntityTypeOption[]
  ): EntityTypeOption[] => {
    const lower = term.toLowerCase();
    const filtered = entityTypes.filter((t) => {
      return (
        t.title.toLowerCase().search(lower) !== -1 ||
        t.value?.toLowerCase().search(lower) !== -1
      );
    });
    return filtered;
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
          <MultiSelectToggleableTags<EntityTypeOption>
            selectionPlaceholder={"All types."}
            defaultItems={entityTypes}
            initialSelection={
              initialFilter.entityTypes?.map((t) => entityTypeLookup[t]) ?? []
            }
            buildTitle={(v) => v.title}
            searchSync={searchType}
            onChange={(types) => {
              props.setFilter((old) => ({
                ...old,
                entityTypes: types.map((t) => t.value),
              }));
            }}
          />
        </div>
      </FieldHorizontal>

      <FieldHorizontal label="Tags" smallLabel>
        <div class="control">
          <TagEntitySelector
            initialSelection={initialFilter.tags}
            onChange={(tags) => {
              props.setFilter((old) => ({
                ...old,
                tags: tags.map((t) => t[FACTOR_ID]),
              }));
            }}
          />
        </div>
      </FieldHorizontal>

      <FieldHorizontal label="Sort" smallLabel>
        <div class="control">
          <SortSelector
            initialSelection={
              initialFilter.sortOrder && initialFilter.sortAttr
                ? [initialFilter.sortAttr, initialFilter.sortOrder]
                : undefined
            }
            onSelected={(x) => {
              const sortAttr = x?.[0][FACTOR_IDENT];
              const sortOrder = x?.[1];
              props.setFilter((f) => ({ ...f, sortAttr, sortOrder }));
            }}
          />
        </div>
      </FieldHorizontal>
    </div>
  );
}
