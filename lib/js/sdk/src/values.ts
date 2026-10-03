import type {
  EncodedTaggedValue,
  IntegerTag,
  SemanticMap,
  SemanticObject,
  SemanticValue,
  SemanticVariant,
  TaggedValue,
  TaggedScalar,
} from "./types.js";

// Decoded objects may use the same field names as explicit wire wrappers.
const exactObjects = new WeakSet<object>();

const safe = (n: bigint): number | bigint =>
  n <= BigInt(Number.MAX_SAFE_INTEGER) && n >= BigInt(Number.MIN_SAFE_INTEGER)
    ? Number(n)
    : n;
const integerRanges: Record<IntegerTag, readonly [bigint, bigint]> = {
  i8: [-128n, 127n],
  i16: [-32768n, 32767n],
  i32: [-2147483648n, 2147483647n],
  i64: [-(2n ** 63n), 2n ** 63n - 1n],
  i128: [-(2n ** 127n), 2n ** 127n - 1n],
  u8: [0n, 255n],
  u16: [0n, 65535n],
  u32: [0n, 4294967295n],
  u64: [0n, 2n ** 64n - 1n],
  u128: [0n, 2n ** 128n - 1n],
};

const checkedInteger = (tag: IntegerTag, value: number | bigint): bigint => {
  if (typeof value !== "number" && typeof value !== "bigint")
    throw new TypeError(`${tag} requires an integer number or bigint`);
  if (typeof value === "number" && !Number.isSafeInteger(value))
    throw new RangeError(`${tag} requires a safe integer number or bigint`);
  const integer = BigInt(value);
  const [min, max] = integerRanges[tag];
  if (integer < min || integer > max)
    throw new RangeError(`${integer} is outside the ${tag} range`);
  return integer;
};

const explicit = <T extends TaggedValue>(
  tagged: T,
): { readonly $tagged: T } => ({
  $tagged: tagged,
});
const one = (value: TaggedValue): [string, unknown] => {
  if (typeof value === "string") return [value, undefined];
  const entries = Object.entries(value);
  if (entries.length !== 1)
    throw new TypeError(
      "tagged Semantic value must contain exactly one variant",
    );
  return entries[0]!;
};

/** Whole milliseconds (truncated toward zero) of a tagged duration: a
 * `[seconds, nanoseconds]` pair, or the legacy integer millisecond form. */
const durationMs = (raw: unknown): number | bigint => {
  if (!Array.isArray(raw))
    return safe(checkedInteger("i64", raw as number | bigint));
  if (raw.length !== 2)
    throw new TypeError("duration requires a [seconds, nanoseconds] pair");
  const seconds = checkedInteger("i64", raw[0] as number | bigint);
  const nanos = checkedInteger("i32", raw[1] as number | bigint);
  return safe(seconds * 1000n + nanos / 1_000_000n);
};

export function decodeTagged(value: TaggedValue): SemanticValue | undefined {
  const [tag, raw] = one(value);
  if (tag === "void") return undefined;
  if (tag === "null") return null;
  if (Object.hasOwn(integerRanges, tag)) {
    const integer = checkedInteger(tag as IntegerTag, raw as number | bigint);
    return ["i8", "i16", "i32", "u8", "u16", "u32"].includes(tag)
      ? Number(integer)
      : safe(integer);
  }
  if (tag === "duration") return durationMs(raw);
  if (tag === "time")
    return safe(checkedInteger("i64", raw as number | bigint));
  if (tag === "date_time")
    return safe(checkedInteger("i128", raw as number | bigint));
  if (tag === "date")
    return Number(checkedInteger("i32", raw as number | bigint));
  if (tag === "f32" || tag === "f64") {
    if (typeof raw !== "number" || !Number.isFinite(raw))
      throw new TypeError(`${tag} requires a finite number`);
    return raw;
  }
  if (tag === "bool") {
    if (typeof raw !== "boolean")
      throw new TypeError("bool requires a boolean");
    return raw;
  }
  if (tag === "string" || tag === "uuid" || tag === "ip_addr") {
    if (typeof raw !== "string")
      throw new TypeError(`${tag} requires a string`);
    return raw;
  }
  if (tag === "bytes") {
    if (!Array.isArray(raw)) throw new TypeError("bytes requires an array");
    return Uint8Array.from(
      raw.map((item) => Number(checkedInteger("u8", item as number | bigint))),
    );
  }
  if (tag === "list") {
    if (!Array.isArray(raw)) throw new TypeError("list requires an array");
    return (raw as TaggedValue[]).map(decodeTagged) as SemanticValue[];
  }
  if (tag === "map") {
    if (!Array.isArray(raw)) throw new TypeError("map requires an array");
    return new Map(
      (raw as [TaggedValue, TaggedValue][]).map((entry) => {
        if (!Array.isArray(entry) || entry.length !== 2)
          throw new TypeError("map entries must be pairs");
        return [decodeTagged(entry[0]), decodeTagged(entry[1])] as [
          SemanticValue,
          SemanticValue,
        ];
      }),
    ) as SemanticMap;
  }
  if (tag === "object") {
    if (!raw || typeof raw !== "object" || Array.isArray(raw))
      throw new TypeError("object requires a JSON object");
    const out = Object.create(null) as SemanticObject;
    for (const [key, item] of Object.entries(
      raw as Record<string, TaggedValue>,
    ))
      Object.defineProperty(out, key, {
        value: decodeTagged(item),
        enumerable: true,
        configurable: true,
        writable: true,
      });
    return out;
  }
  if (tag === "variant") {
    if (!raw || typeof raw !== "object" || Array.isArray(raw))
      throw new TypeError("variant requires an object");
    const v = raw as { type?: string; variant: string; value: TaggedValue };
    if (
      typeof v.variant !== "string" ||
      (v.type !== undefined && typeof v.type !== "string") ||
      !("value" in v)
    )
      throw new TypeError("invalid variant value");
    return {
      $variant: v.variant,
      value: decodeTagged(v.value) as SemanticValue,
      ...(v.type === undefined ? {} : { type: v.type }),
    };
  }
  throw new TypeError(`unknown Semantic value tag '${tag}'`);
}

/** Decode containers retaining scalar tags. primitiveScalars unwraps losslessly representable
 * null/bool/string values for typed RPC records; other scalar tags remain exact. */
export function decodeTaggedExact(
  value: TaggedValue,
  primitiveScalars = false,
): SemanticValue {
  const [tag, raw] = one(value);
  if (primitiveScalars && ["null", "bool", "string"].includes(tag))
    return decodeTagged(value);
  if (tag === "list") {
    if (!Array.isArray(raw)) throw new TypeError("list requires an array");
    return (raw as TaggedValue[]).map((entry) =>
      decodeTaggedExact(entry, primitiveScalars),
    );
  }
  if (tag === "map") {
    if (!Array.isArray(raw)) throw new TypeError("map requires an array");
    return new Map(
      (raw as [TaggedValue, TaggedValue][]).map((entry) => {
        if (!Array.isArray(entry) || entry.length !== 2)
          throw new TypeError("map entries must be pairs");
        return [
          decodeTaggedExact(entry[0], primitiveScalars),
          decodeTaggedExact(entry[1], primitiveScalars),
        ];
      }),
    );
  }
  if (tag === "object") {
    if (!raw || typeof raw !== "object" || Array.isArray(raw))
      throw new TypeError("object requires a JSON object");
    const out = Object.create(null) as SemanticObject;
    for (const [key, item] of Object.entries(
      raw as Record<string, TaggedValue>,
    ))
      Object.defineProperty(out, key, {
        value: decodeTaggedExact(item, primitiveScalars),
        enumerable: true,
        configurable: true,
        writable: true,
      });
    exactObjects.add(out);
    return out;
  }
  if (tag === "variant") {
    if (!raw || typeof raw !== "object" || Array.isArray(raw))
      throw new TypeError("variant requires an object");
    const variant = raw as {
      type?: string;
      variant: string;
      value: TaggedValue;
    };
    if (
      typeof variant.variant !== "string" ||
      (variant.type !== undefined && typeof variant.type !== "string") ||
      !("value" in variant)
    )
      throw new TypeError("invalid variant value");
    return {
      $variant: variant.variant,
      value: decodeTaggedExact(variant.value, primitiveScalars),
      ...(variant.type === undefined ? {} : { type: variant.type }),
    };
  }
  decodeTagged(value);
  return explicit(value);
}

export function encodeTagged(
  value: SemanticValue | undefined,
  integerTag: IntegerTag = "i64",
): TaggedValue {
  if (value === undefined) return "void";
  if (value === null) return "null";
  if (typeof value === "boolean") return { bool: value };
  if (typeof value === "bigint")
    return { [integerTag]: checkedInteger(integerTag, value) } as TaggedValue;
  if (typeof value === "number") {
    if (!Number.isFinite(value))
      throw new TypeError("Semantic numbers must be finite");
    return Number.isInteger(value)
      ? { i64: checkedInteger("i64", value) }
      : { f64: value };
  }
  if (typeof value === "string") return { string: value };
  if (value instanceof Uint8Array) return { bytes: [...value] };
  if (value instanceof Date)
    return {
      date_time: checkedInteger("i128", BigInt(value.getTime()) * 1_000_000n),
    };
  if (Array.isArray(value))
    return { list: value.map((item) => encodeTagged(item, integerTag)) };
  if (value instanceof Map)
    return {
      map: [...value].map(([key, item]) => [
        encodeTagged(key, integerTag),
        encodeTagged(item, integerTag),
      ]),
    };
  if ("$tagged" in value && !exactObjects.has(value))
    return (value as EncodedTaggedValue).$tagged;
  if ("$variant" in value && !exactObjects.has(value)) {
    const v = value as SemanticVariant;
    return {
      variant: {
        variant: v.$variant,
        value: encodeTagged(v.value, integerTag),
        ...(v.type === undefined ? {} : { type: v.type }),
      },
    };
  }
  const fields = Object.create(null) as Record<string, TaggedValue>;
  for (const [key, item] of Object.entries(value))
    if (item !== undefined)
      Object.defineProperty(fields, key, {
        value: encodeTagged(item, integerTag),
        enumerable: true,
        configurable: true,
        writable: true,
      });
  return { object: fields };
}

/** Constructors for widths or scalar kinds that cannot be inferred from JavaScript. */
export const value = {
  void: (): EncodedTaggedValue => explicit("void"),
  null: (): EncodedTaggedValue => explicit("null"),
  int: <T extends IntegerTag>(tag: T, n: number | bigint): TaggedScalar<T> =>
    explicit({
      [tag]: checkedInteger(tag, n),
    } as TaggedValue) as TaggedScalar<T>,
  uuid: (v: string): TaggedScalar<"uuid"> => explicit({ uuid: v }),
  ipAddr: (v: string): TaggedScalar<"ip_addr"> => explicit({ ip_addr: v }),
  durationMs: (v: number | bigint): TaggedScalar<"duration"> =>
    explicit({ duration: checkedInteger("i64", v) }),
  timeNanos: (v: number | bigint): TaggedScalar<"time"> =>
    explicit({ time: checkedInteger("i64", v) }),
  dateJulianDay: (v: number): TaggedScalar<"date"> => {
    if (!Number.isInteger(v) || v < -2147483648 || v > 2147483647)
      throw new RangeError("date requires an i32 Julian day");
    return explicit({ date: v });
  },
  dateTimeNanos: (v: number | bigint): TaggedScalar<"date_time"> =>
    explicit({ date_time: checkedInteger("i128", v) }),
  variant: (
    variant: string,
    v: SemanticValue,
    type?: string,
  ): EncodedTaggedValue =>
    explicit({
      variant: {
        variant,
        value: encodeTagged(v),
        ...(type === undefined ? {} : { type }),
      },
    }),
};
