import type {
  ContractConstant,
  Package,
  SemanticObject,
  SemanticValue,
  TaggedValue,
} from "./types.js";
import { decodeTagged, encodeTagged } from "./values.js";

/** Lossless JSON for RPC envelopes. JSON integer tokens decode to bigint when unsafe. */
export function stringifyJson(value: unknown): string {
  const seen = new Set<object>();
  const write = (v: unknown): string => {
    if (v === null) return "null";
    if (typeof v === "string") return JSON.stringify(v);
    if (typeof v === "boolean") return String(v);
    if (typeof v === "bigint") return v.toString();
    if (typeof v === "number") {
      if (!Number.isFinite(v))
        throw new TypeError("cannot encode non-finite JSON number");
      return String(v);
    }
    if (typeof v !== "object")
      throw new TypeError(`cannot encode ${typeof v} as JSON`);
    if (seen.has(v)) throw new TypeError("cannot encode cyclic JSON");
    seen.add(v);
    const result = Array.isArray(v)
      ? `[${v.map(write).join(",")}]`
      : `{${Object.entries(v)
          .filter(([, x]) => x !== undefined)
          .map(([k, x]) => `${JSON.stringify(k)}:${write(x)}`)
          .join(",")}}`;
    seen.delete(v);
    return result;
  };
  return write(value);
}

export function parseJson(text: string): unknown {
  let i = 0;
  const ws = () => {
    while (/\s/.test(text[i] ?? "")) i++;
  };
  const parse = (): unknown => {
    ws();
    const c = text[i];
    if (c === '"') {
      const start = i++;
      while (i < text.length) {
        if (text[i] === "\\") i += 2;
        else if (text[i++] === '"') return JSON.parse(text.slice(start, i));
      }
      throw new SyntaxError("unterminated JSON string");
    }
    if (c === "[") {
      i++;
      const out: unknown[] = [];
      ws();
      if (text[i] === "]") {
        i++;
        return out;
      }
      for (;;) {
        out.push(parse());
        ws();
        if (text[i++] === "]") return out;
        if (text[i - 1] !== ",")
          throw new SyntaxError(`expected ',' at ${i - 1}`);
      }
    }
    if (c === "{") {
      i++;
      const out: Record<string, unknown> = Object.create(null) as Record<
        string,
        unknown
      >;
      ws();
      if (text[i] === "}") {
        i++;
        return out;
      }
      for (;;) {
        ws();
        const key = parse();
        if (typeof key !== "string")
          throw new SyntaxError(`expected object key at ${i}`);
        ws();
        if (text[i++] !== ":")
          throw new SyntaxError(`expected ':' at ${i - 1}`);
        Object.defineProperty(out, key, {
          value: parse(),
          enumerable: true,
          configurable: true,
          writable: true,
        });
        ws();
        if (text[i++] === "}") return out;
        if (text[i - 1] !== ",")
          throw new SyntaxError(`expected ',' at ${i - 1}`);
      }
    }
    for (const [token, value] of [
      ["true", true],
      ["false", false],
      ["null", null],
    ] as const)
      if (text.startsWith(token, i)) {
        i += token.length;
        return value;
      }
    const match = /^-?(?:0|[1-9]\d*)(?:\.\d+)?(?:[eE][+-]?\d+)?/.exec(
      text.slice(i),
    );
    if (!match) throw new SyntaxError(`invalid JSON at ${i}`);
    i += match[0].length;
    if (/[.eE]/.test(match[0])) return Number(match[0]);
    const n = BigInt(match[0]);
    return n <= BigInt(Number.MAX_SAFE_INTEGER) &&
      n >= BigInt(Number.MIN_SAFE_INTEGER)
      ? Number(n)
      : n;
  };
  const value = parse();
  ws();
  if (i !== text.length) throw new SyntaxError(`trailing JSON at ${i}`);
  return value;
}

/** Lossless JSON used only for externally-tagged RPC envelopes. */
export const rpcJson = { stringify: stringifyJson, parse: parseJson };

/** Facet's externally-tagged representation of a standalone Semantic Value. */
export const facetValueJson = {
  stringify(value: SemanticValue | undefined): string {
    return stringifyJson(encodeTagged(value));
  },
  parse(text: string): SemanticValue | undefined {
    return decodeTagged(parseJson(text) as TaggedValue);
  },
};

/** Ordinary, untagged JSON used by the HTTP file API. */
const flatValue = (value: unknown): unknown => {
  if (
    value === undefined ||
    value === null ||
    typeof value === "boolean" ||
    typeof value === "number" ||
    typeof value === "bigint" ||
    typeof value === "string"
  )
    return value ?? null;
  if (value instanceof Uint8Array) return [...value];
  if (value instanceof Date) return value.toISOString();
  if (Array.isArray(value)) return value.map(flatValue);
  if (value instanceof Map)
    return Object.fromEntries(
      [...value].map(([key, item]) => [String(key), flatValue(item)]),
    );
  if (typeof value === "object" && "$variant" in value) {
    const variant = value as {
      $variant: string;
      value: unknown;
      type?: string;
    };
    return {
      ...(variant.type === undefined ? {} : { type: variant.type }),
      variant: variant.$variant,
      value: flatValue(variant.value),
    };
  }
  if (typeof value === "object" && "$tagged" in value)
    throw new TypeError(
      "explicit tagged values cannot be encoded as flat JSON",
    );
  return Object.fromEntries(
    Object.entries(value as Record<string, unknown>)
      .filter(([, item]) => item !== undefined)
      .map(([key, item]) => [key, flatValue(item)]),
  );
};
export const flatJson = {
  stringify(value: unknown): string {
    return stringifyJson(flatValue(value));
  },
  parse<T = unknown>(text: string): T {
    return parseJson(text) as T;
  },
};

const visitConstants = (
  constants: Record<string, ContractConstant>,
  encode: boolean,
): void => {
  for (const constant of Object.values(constants)) {
    visitMeta(constant.meta, encode);
    visitType(constant.ty, encode);
    constant.value = (
      encode
        ? encodeTagged(constant.value)
        : decodeTagged(constant.value as TaggedValue)
    ) as SemanticValue;
  }
};

const visitMeta = (value: unknown, encode: boolean): void => {
  if (!value || typeof value !== "object") return;
  const meta = value as Record<string, unknown>;
  if (Array.isArray(meta.examples))
    meta.examples = meta.examples.map((item) =>
      encode
        ? encodeTagged(item as SemanticValue)
        : decodeTagged(item as TaggedValue),
    );
};

const visitValue = (
  holder: Record<string, unknown>,
  key: string,
  encode: boolean,
): void => {
  holder[key] = encode
    ? encodeTagged(holder[key] as SemanticValue)
    : decodeTagged(holder[key] as TaggedValue);
};

/** This is only called at statically expression-typed schema slots. */
const visitSchemaExpression = (value: unknown, encode: boolean): void => {
  if (!value || typeof value !== "object") return;
  if (Array.isArray(value)) {
    value.forEach((item) => visitSchemaExpression(item, encode));
    return;
  }
  const expression = value as Record<string, unknown>;
  if (Object.keys(expression).length === 1 && "literal" in expression) {
    const literal = expression.literal;
    if (literal && typeof literal === "object" && "value" in literal)
      visitValue(literal as Record<string, unknown>, "value", encode);
    return;
  }
  Object.values(expression).forEach((item) =>
    visitSchemaExpression(item, encode),
  );
};

const visitConstraint = (value: unknown, encode: boolean): void => {
  if (!value || typeof value !== "object") return;
  const constraint = value as Record<string, unknown>;
  if (Object.keys(constraint).length !== 1) return;
  if ("contains" in constraint) visitValue(constraint, "contains", encode);
  else if ("default_value" in constraint) {
    const holder = constraint.default_value;
    if (holder && typeof holder === "object" && "value" in holder)
      visitValue(holder as Record<string, unknown>, "value", encode);
  } else if ("default_expr" in constraint) {
    const holder = constraint.default_expr;
    if (holder && typeof holder === "object")
      visitSchemaExpression((holder as Record<string, unknown>).expr, encode);
  }
};

const visitFunction = (value: unknown, encode: boolean): void => {
  if (!value || typeof value !== "object") return;
  const fn = value as Record<string, any>;
  for (const param of fn.params ?? []) visitType(param.ty, encode);
  for (const result of fn.results ?? []) visitType(result, encode);
  if (fn.throws) visitType(fn.throws, encode);
};

const visitInterface = (value: unknown, encode: boolean): void => {
  if (!value || typeof value !== "object") return;
  for (const method of (value as Record<string, any>).methods ?? [])
    visitFunction(method.signature, encode);
};

const visitRecord = (value: unknown, encode: boolean): void => {
  if (!value || typeof value !== "object") return;
  const record = value as Record<string, any>;
  for (const field of Object.values(record.fields ?? {}) as Array<
    Record<string, any>
  >) {
    visitMeta(field.meta, encode);
    visitType(field.ty, encode);
    if (field.default !== null && field.default !== undefined)
      visitValue(field, "default", encode);
  }
  if (record.additional) visitType(record.additional, encode);
};

const visitAttribute = (value: unknown, encode: boolean): void => {
  if (!value || typeof value !== "object") return;
  const attribute = value as Record<string, any>;
  visitMeta(attribute.meta, encode);
  visitType(attribute.ty, encode);
  for (const constraint of attribute.constraints ?? [])
    visitConstraint(constraint, encode);
};

const visitClass = (value: unknown, encode: boolean): void => {
  if (!value || typeof value !== "object") return;
  const klass = value as Record<string, any>;
  visitMeta(klass.meta, encode);
  for (const field of Object.values(klass.attributes ?? {}) as Array<
    Record<string, any>
  >) {
    visitMeta(field.meta, encode);
    for (const constraint of field.constraints ?? [])
      visitConstraint(constraint, encode);
    if (field.computed) visitSchemaExpression(field.computed, encode);
  }
  for (const constraint of klass.constraints ?? []) {
    if (constraint && typeof constraint === "object" && "field" in constraint)
      visitConstraint(
        (constraint as Record<string, any>).field?.constraint,
        encode,
      );
    else if (
      constraint &&
      typeof constraint === "object" &&
      "multi_field_expr" in constraint
    )
      visitSchemaExpression(
        (constraint as Record<string, any>).multi_field_expr?.expr,
        encode,
      );
  }
};

const visitType = (value: unknown, encode: boolean): void => {
  if (!value || typeof value !== "object") return;
  const node = value as Record<string, any>;
  for (const constraint of node.constraints ?? [])
    visitConstraint(constraint, encode);
  const kind = node.kind;
  if (!kind || typeof kind !== "object") return;
  const entry = Object.entries(kind)[0] as [string, any] | undefined;
  if (!entry) return;
  const [name, body] = entry;
  if (["optional", "array", "list", "set"].includes(name))
    visitType(body?.inner ?? body?.items, encode);
  else if (name === "tuple") {
    for (const item of body?.items ?? []) visitType(item, encode);
    if (body?.rest) visitType(body.rest, encode);
  } else if (name === "map") {
    visitType(body?.keys, encode);
    visitType(body?.values, encode);
  } else if (name === "record") visitRecord(body, encode);
  else if (name === "attribute") visitAttribute(body, encode);
  else if (name === "class") visitClass(body, encode);
  else if (name === "union" || name === "intersection")
    for (const item of body?.variants ?? []) visitType(item, encode);
  else if (name === "result") {
    visitType(body?.ok, encode);
    visitType(body?.err, encode);
  } else if (name === "variant") {
    for (const variant of body?.variants ?? []) {
      visitMeta(variant.meta, encode);
      if (variant.discriminant !== null && variant.discriminant !== undefined)
        visitValue(variant, "discriminant", encode);
      const variantPayload = variant.payload;
      if (variantPayload && typeof variantPayload === "object") {
        if ("tuple" in variantPayload)
          for (const item of variantPayload.tuple ?? [])
            visitType(item, encode);
        else if ("record" in variantPayload)
          visitRecord(variantPayload.record, encode);
        else if ("newtype" in variantPayload)
          visitType(variantPayload.newtype, encode);
      }
    }
  } else if (name === "enum")
    for (const variant of body?.variants ?? []) visitMeta(variant.meta, encode);
  else if (name === "function") visitFunction(body, encode);
  else if (name === "interface") visitInterface(body, encode);
  else if (name === "stream") {
    visitType(body?.element, encode);
    if (body?.end) visitType(body.end, encode);
  } else if (name === "handle" || name === "ref")
    for (const arg of (name === "handle"
      ? body?.interface?.args
      : body?.args) ?? [])
      visitType(arg, encode);
};

const visitTypeDef = (value: unknown, encode: boolean): void => {
  if (!value || typeof value !== "object") return;
  const definition = value as Record<string, any>;
  visitMeta(definition.meta, encode);
  for (const param of definition.params ?? []) {
    for (const bound of param.bounds ?? [])
      for (const arg of bound.args ?? []) visitType(arg, encode);
    if (param.default) visitType(param.default, encode);
  }
  visitType(definition.ty, encode);
};

/** This is only called at statically database-query-typed slots. */
const visitQuery = (value: unknown, encode: boolean): void => {
  const walk = (item: unknown): void => {
    if (!item || typeof item !== "object") return;
    if (Array.isArray(item)) {
      item.forEach(walk);
      return;
    }
    const record = item as Record<string, unknown>;
    if (Object.keys(record).length === 1 && "operand" in record) {
      const operand = record.operand;
      if (operand && typeof operand === "object" && "literal" in operand) {
        visitValue(operand as Record<string, unknown>, "literal", encode);
        return;
      }
    }
    Object.values(record).forEach(walk);
  };
  walk(value);
};

const visitDdl = (value: unknown, encode: boolean): void => {
  if (!value || typeof value !== "object") return;
  const operation = value as Record<string, any>;
  if ("upsert_attribute" in operation)
    visitAttribute(operation.upsert_attribute?.attribute, encode);
  else if ("upsert_type_def" in operation)
    visitTypeDef(operation.upsert_type_def?.type_def, encode);
  else if ("upsert_record_type" in operation)
    visitRecord(operation.upsert_record_type?.record, encode);
  else if ("upsert_class" in operation)
    visitClass(operation.upsert_class?.class, encode);
  else if ("upsert_relationship" in operation)
    visitMeta(operation.upsert_relationship?.relationship?.meta, encode);
};

/** Facet JSON for Package, whose embedded Value fields remain externally tagged. */
export const packageJson = {
  stringify(pkg: Package): string {
    const copy = structuredClone(pkg) as Package;
    visitMeta(copy.meta, true);
    const visitModule = (module: Package["root"]): void => {
      visitMeta(module.meta, true);
      Object.values(module.types).forEach((item) => visitTypeDef(item, true));
      Object.values(module.attributes).forEach((item) =>
        visitAttribute(item, true),
      );
      Object.values(module.classes).forEach((item) => visitClass(item, true));
      Object.values(module.interfaces).forEach((item) =>
        visitInterface(item, true),
      );
      visitConstants(module.constants, true);
      for (const contract of Object.values(module.contracts)) {
        visitMeta(contract.meta, true);
        Object.values(contract.types).forEach((item) =>
          visitTypeDef(item, true),
        );
        Object.values(contract.attributes).forEach((item) =>
          visitAttribute(item, true),
        );
        Object.values(contract.classes).forEach((item) =>
          visitClass(item, true),
        );
        Object.values(contract.interfaces).forEach((item) => {
          visitMeta(item.meta, true);
          visitInterface(item.interface, true);
        });
        Object.values(contract.functions).forEach((item) => {
          visitMeta(item.meta, true);
          visitFunction(item.signature, true);
        });
        visitConstants(contract.constants, true);
      }
    };
    visitModule(copy.root);
    Object.values(copy.modules).forEach(visitModule);
    for (const migration of copy.migrations) {
      visitMeta(migration.meta, true);
      for (const operation of migration.operations) {
        if ("insert" in operation)
          operation.insert.object = Object.fromEntries(
            Object.entries(operation.insert.object).map(([key, item]) => [
              key,
              encodeTagged(item),
            ]),
          ) as unknown as SemanticObject;
        else if ("ddl" in operation) visitDdl(operation.ddl, true);
        else visitQuery(operation, true);
      }
    }
    return stringifyJson(copy);
  },
  parse(text: string): Package {
    const copy = parseJson(text) as Package;
    visitMeta(copy.meta, false);
    const visitModule = (module: Package["root"]): void => {
      visitMeta(module.meta, false);
      Object.values(module.types).forEach((item) => visitTypeDef(item, false));
      Object.values(module.attributes).forEach((item) =>
        visitAttribute(item, false),
      );
      Object.values(module.classes).forEach((item) => visitClass(item, false));
      Object.values(module.interfaces).forEach((item) =>
        visitInterface(item, false),
      );
      visitConstants(module.constants, false);
      for (const contract of Object.values(module.contracts)) {
        visitMeta(contract.meta, false);
        Object.values(contract.types).forEach((item) =>
          visitTypeDef(item, false),
        );
        Object.values(contract.attributes).forEach((item) =>
          visitAttribute(item, false),
        );
        Object.values(contract.classes).forEach((item) =>
          visitClass(item, false),
        );
        Object.values(contract.interfaces).forEach((item) => {
          visitMeta(item.meta, false);
          visitInterface(item.interface, false);
        });
        Object.values(contract.functions).forEach((item) => {
          visitMeta(item.meta, false);
          visitFunction(item.signature, false);
        });
        visitConstants(contract.constants, false);
      }
    };
    visitModule(copy.root);
    Object.values(copy.modules).forEach(visitModule);
    for (const migration of copy.migrations) {
      visitMeta(migration.meta, false);
      for (const operation of migration.operations) {
        if ("insert" in operation)
          operation.insert.object = Object.fromEntries(
            Object.entries(operation.insert.object).map(([key, item]) => [
              key,
              decodeTagged(item as TaggedValue) as SemanticValue,
            ]),
          );
        else if ("ddl" in operation) visitDdl(operation.ddl, false);
        else visitQuery(operation, false);
      }
    }
    return copy;
  },
};

/** @deprecated Use packageJson, facetValueJson, or flatJson for the intended wire domain. */
export const facetJson = packageJson;
