import { JSX } from "solid-js";
import { newSelect } from "semantic/dist/api";
import { useApi, useRegistry } from "../../context";
import { Expr, Select } from "semantic/dist/core";
import {
  exprAnd,
  exprAttr,
  exprRegexIMatch,
  exprEq,
  exprLiteral,
  exprOr,
} from "semantic/dist/db";
import { ValueMap } from "../../semantic/registry";
import { FACTOR_ID, SEMANTIC_TITLE } from "semantic/dist/schema";
import { Button } from "../bulma/button";
import { SearchSelect } from "../util/SearchSelect";

export interface EntityPickerProps {
  autoFocus?: boolean;

  baseFilter?: Expr;
  buildFilter?: (term: string) => Expr;
  buildSelect?: (term: string) => Select;

  renderItem?: (
    item: ValueMap,
    index: number,
    onClick: () => void
  ) => JSX.Element;
  renderItemLabel?: (item: ValueMap) => JSX.Element;

  onSelect: (entity: ValueMap) => void;
}

export function EntityPicker(props: EntityPickerProps) {
  const reg = useRegistry();

  let renderItem;
  if (props.renderItem) {
    renderItem = props.renderItem;
  } else {
    const renderLabel =
      props.renderItemLabel || ((item: ValueMap) => reg.entityTitle(item));
    renderItem = (item: ValueMap, _index: number, onClick: () => void) => {
      return (
        <div>
          <Button onClick={onClick}>{renderLabel(item)}</Button>
        </div>
      );
    };
  }

  return (
    <SearchSelect<ValueMap>
      autoFocus={props.autoFocus}
      search={(term) => {
        term = term.trim();
        if (term.length < 1) {
          return Promise.resolve([]);
        }
        const termExpr = exprOr(
          exprEq(exprAttr(FACTOR_ID), exprLiteral(term)),
          exprRegexIMatch(exprAttr(SEMANTIC_TITLE), term)
        );

        let select: Select;

        if (props.buildSelect) {
          select = props.buildSelect(term);
        } else if (props.buildFilter) {
          const expr = props.baseFilter
            ? exprAnd(props.baseFilter, props.buildFilter(term))
            : props.buildFilter(term);
          select = {
            ...newSelect(),
            filter: expr,
            // TODO: don't use bigint in type!
            limit: 10 as any,
          };
        } else {
          const expr = props.baseFilter
            ? exprAnd(props.baseFilter, termExpr)
            : termExpr;
          select = {
            ...newSelect(),
            filter: expr,
            // TODO: don't use bigint in type!
            limit: 10 as any,
          };
        }

        return useApi().select(select);
      }}
      onSelect={props.onSelect}
      renderItem={renderItem}
    />
  );
}
