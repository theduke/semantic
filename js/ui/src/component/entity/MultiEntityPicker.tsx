import { ValueMap } from "semantic/dist/api";
import { Expr } from "semantic/dist/core";
import { DeletableTag, Tag, Tags } from "solid-bulma";
import { For, JSX } from "solid-js";
import { searchEntities } from ".";
import { useApi, useRegistry } from "../../context";
import { MultiSelectSearch } from "../util/MultiSelectSearch";

export interface MultiEntityPickerProps {
  baseFilter?: Expr;
  onChange: (newEntities: ValueMap[]) => void;
  initialSelection?: ValueMap[];
}

export function MultiEntityPicker(props: MultiEntityPickerProps): JSX.Element {
  const api = useApi();
  const reg = useRegistry();

  const search = (term: string): Promise<ValueMap[]> =>
    searchEntities(api, term, 50, props.baseFilter ?? null);

  return (
    <MultiSelectSearch<ValueMap>
      search={search}
      onChange={props.onChange}
      initialSelection={props.initialSelection ?? []}
      renderItem={(item, onSelect) => {
        const title = reg.entityTitle(item);
        return (
          <Tag style={{ cursor: "pointer" }} onclick={onSelect}>
            {title}
          </Tag>
        );
      }}
      renderItemWrapper={(items) => <Tags>{items}</Tags>}
      renderSelected={(items, onRemove) => {
        return (
          <div class="m-2">
            <Tags>
              <For each={items()}>
                {(item, index) => {
                  return (
                    <DeletableTag onDelete={() => onRemove(index())}>
                      {reg.entityTitle(item)}
                    </DeletableTag>
                  );
                }}
              </For>
            </Tags>
          </div>
        );
      }}
    />
  );
}
