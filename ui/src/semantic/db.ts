import { BinaryOp, Expr, Value } from "./core";
import { FACTOR_TYPE } from "./schema";

export function exprBinary(op: BinaryOp, left: Expr, right: Expr): Expr {
  return { BinaryOp: { op, left, right } };
}

export function exprEq(left: Expr, right: Expr): Expr {
  return exprBinary("Eq", left, right);
}

export function exprAttr(attrName: string): Expr {
  return { Attr: attrName };
}

export function exprLiteral(value: Value): Expr {
  return { Literal: value };
}

export function exprIsEntityType(ty: string): Expr {
  return exprEq(exprAttr(FACTOR_TYPE), exprLiteral(ty));
}
