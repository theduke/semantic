import { BinaryOp, Expr, UnaryOp, Value } from "./core";
import { FACTOR_TYPE } from "./schema";

export function exprBinary(op: BinaryOp, left: Expr, right: Expr): Expr {
  return { BinaryOp: { op, left, right } };
}

export function exprEq(left: Expr, right: Expr): Expr {
  return exprBinary("Eq", left, right);
}

export function exprNotEq(left: Expr, right: Expr): Expr {
  return exprBinary("Neq", left, right);
}

export function exprAnd(left: Expr, right: Expr): Expr {
  return exprBinary("And", left, right);
}

export function exprUnary(op: UnaryOp, expr: Expr): Expr {
  return { UnaryOp: { op, expr } };
}

export function exprNot(inner: Expr): Expr {
  return { UnaryOp: { op: "Not", expr: inner } }
}

export function exprList(items: Expr[]): Expr {
  return { List: items };
}

// Build nested AND expressions from a list.
// Returns null if the list is empty, or the single expression if lenght is 1.
export function exprAndMany(exprs: Expr[]): Expr | null {
  if (exprs.length === 0) {
    return null;
  } else if (exprs.length < 2) {
    return exprs[0];
  } else {
    let ands = exprAnd(exprs[0], exprs[1]);

    for (const expr of exprs.slice(2)) {
      ands = exprAnd(ands, expr);
    }

    return ands;
  }
}

export function exprOr(left: Expr, right: Expr): Expr {
  return exprBinary("Or", left, right);
}

export function exprContains(left: Expr, right: Expr): Expr {
  return exprBinary("Contains", left, right);
}

export function exprRegexMatch(left: Expr, regex: string): Expr {
  return exprBinary("RegexMatch", left, exprLiteral(regex));
}

// Case-insensitive regex match.
export function exprRegexIMatch(left: Expr, regex: string): Expr {
  return exprBinary("RegexMatchCaseInsensitive", left, exprLiteral(regex));
}

export function exprIn(left: Expr, right: Expr): Expr {
  return exprBinary("In", left, right);
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

export function exprIsInEntityTypes(types: string[]): Expr {
  return exprIn(exprAttr(FACTOR_TYPE), exprLiteral(types));
}
