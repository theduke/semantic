import { newSelect, ValueMap } from "semantic/dist/api";
import { Expr, Select } from "semantic/dist/core";
import {
  exprAnd,
  exprAttr,
  exprEq,
  exprLiteral,
  exprOr,
  exprRegexIMatch,
} from "semantic/dist/db";
import { FACTOR_ID, SEMANTIC_TITLE } from "semantic/dist/schema";
import { DeletableTag, Tag, Tags } from "solid-bulma";
import { For, JSX } from "solid-js";
import { useApi } from "../../context";
import { MultiSelectSearch } from "../util/MultiSelectSearch";

export interface MultiEntityPickerProps {
  baseFilter?: Expr;
  onChange: (newEntities: ValueMap[]) => void;
  initialSelection?: ValueMap[];
}

export function MultiEntityPicker(props: MultiEntityPickerProps): JSX.Element {
  const api = useApi();

  const search = async (rawTerm: string): Promise<ValueMap[]> => {
    const term = rawTerm.trim().toLowerCase();
    if (term === "") {
      return Promise.resolve([]);
    }

    const searchFilter = exprOr(
      exprEq(exprAttr(FACTOR_ID), exprLiteral(term)),
      exprRegexIMatch(exprAttr(SEMANTIC_TITLE), term)
    );

    const filter = props.baseFilter
      ? exprAnd(props.baseFilter, searchFilter)
      : searchFilter;

    const select: Select = {
      ...newSelect(),
      filter,
      limit: 20 as any,
    };

    return api.select(select);
  };

  return (
    <MultiSelectSearch<ValueMap>
      search={search}
      onChange={props.onChange}
      initialSelection={props.initialSelection ?? []}
      renderItem={(item, onSelect) => {
        const title = item[SEMANTIC_TITLE] ?? item[FACTOR_ID];
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
                      {item[SEMANTIC_TITLE] ?? item[FACTOR_ID]}
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
