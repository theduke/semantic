"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
exports.exprIsInEntityTypes = exports.exprIsEntityType = exports.exprLiteral = exports.exprAttr = exports.exprIn = exports.exprContains = exports.exprOr = exports.exprAndMany = exports.exprAnd = exports.exprNotEq = exports.exprEq = exports.exprBinary = void 0;
const schema_1 = require("./schema");
function exprBinary(op, left, right) {
    return { BinaryOp: { op, left, right } };
}
exports.exprBinary = exprBinary;
function exprEq(left, right) {
    return exprBinary("Eq", left, right);
}
exports.exprEq = exprEq;
function exprNotEq(left, right) {
    return exprBinary("Neq", left, right);
}
exports.exprNotEq = exprNotEq;
function exprAnd(left, right) {
    return exprBinary("And", left, right);
}
exports.exprAnd = exprAnd;
// Build nested AND expressions from a list.
// Returns null if the list is empty, or the single expression if lenght is 1.
function exprAndMany(exprs) {
    if (exprs.length === 0) {
        return null;
    }
    else if (exprs.length < 2) {
        return exprs[0];
    }
    else {
        let ands = exprAnd(exprs[0], exprs[1]);
        for (const expr of exprs.slice(2)) {
            ands = exprAnd(ands, expr);
        }
        return ands;
    }
}
exports.exprAndMany = exprAndMany;
function exprOr(left, right) {
    return exprBinary("Or", left, right);
}
exports.exprOr = exprOr;
function exprContains(left, right) {
    return exprBinary("Contains", left, right);
}
exports.exprContains = exprContains;
function exprIn(left, right) {
    return exprBinary("In", left, right);
}
exports.exprIn = exprIn;
function exprAttr(attrName) {
    return { Attr: attrName };
}
exports.exprAttr = exprAttr;
function exprLiteral(value) {
    return { Literal: value };
}
exports.exprLiteral = exprLiteral;
function exprIsEntityType(ty) {
    return exprEq(exprAttr(schema_1.FACTOR_TYPE), exprLiteral(ty));
}
exports.exprIsEntityType = exprIsEntityType;
function exprIsInEntityTypes(types) {
    return exprIn(exprAttr(schema_1.FACTOR_TYPE), exprLiteral(types));
}
exports.exprIsInEntityTypes = exprIsInEntityTypes;
