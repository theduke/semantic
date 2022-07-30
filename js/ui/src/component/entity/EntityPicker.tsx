import { JSX } from "solid-js";
import { newSelect } from "../../api";
import { useApi, useRegistry } from "../../context";
import { Expr, Select } from "../../semantic/core";
import {
  exprAnd,
  exprAttr,
  exprContains,
  exprEq,
  exprLiteral,
  exprOr,
} from "../../semantic/db";
import { ValueMap } from "../../semantic/registry";
import { FACTOR_ID, SEMANTIC_TITLE } from "../../semantic/schema";
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

  onSelect: (entity: ValueMap) => void;
}

export function EntityPicker(props: EntityPickerProps) {
  const reg = useRegistry();
  const renderItem =
    props.renderItem ??
    ((item, _index, onClick) => {
      return (
        <div>
          <Button onClick={onClick}>{reg.entityTitle(item)}</Button>
        </div>
      );
    });

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
          exprContains(exprAttr(SEMANTIC_TITLE), exprLiteral(term))
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
