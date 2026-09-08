import type {
  ClassType,
  Contract,
  Module,
  Package,
  TypeNode,
} from "../types.js";
import type { CommandModel, PackageModel, TypeModel } from "./model.js";
import { identifier } from "./identifier.js";
const literal = (value: string | number): string =>
  typeof value === "string" ? JSON.stringify(value) : String(value);
const object = (value: unknown): Record<string, any> =>
  value && typeof value === "object" ? (value as Record<string, any>) : {};
const payload = (
  kind: unknown,
  key: string,
): Record<string, any> | undefined => {
  const value = object(kind)[key];
  return value && typeof value === "object"
    ? (value as Record<string, any>)
    : undefined;
};
const isOptional = (node: TypeNode): boolean =>
  typeof node.kind === "object" &&
  node.kind !== null &&
  "optional" in node.kind;

type CompoundContext = "union" | "intersection";

type TypePrecedence = "function" | "union" | "intersection" | "primary";

function typePrecedence(node: TypeNode): TypePrecedence {
  if (node.kind === "uuid" || node.kind === "json") return "primary";
  const kind = object(node.kind);
  const attribute = payload(kind, "attribute");
  if (attribute) return typePrecedence(attribute.ty as TypeNode);
  if ("function" in kind) return "function";
  if (
    "optional" in kind ||
    "union" in kind ||
    "result" in kind ||
    "variant" in kind ||
    "enum" in kind ||
    "temporal" in kind
  )
    return "union";
  const number = object(kind.number);
  if (
    "number" in kind &&
    !("float" in number) &&
    !("decimal" in number) &&
    !("rational" in number) &&
    !("complex" in number) &&
    !(
      "int" in number &&
      ["i8", "i16", "i24", "i32"].includes(String(number.int))
    ) &&
    !(
      ("uint" in number || "u_int" in number) &&
      ["u8", "u16", "u24", "u32"].includes(String(number.uint ?? number.u_int))
    )
  )
    return "union";
  if ("intersection" in kind) return "intersection";
  return "primary";
}

function nestedType(
  node: TypeNode,
  resolveRef: (name: string) => string,
  context: CompoundContext,
): string {
  const type = typeScriptType(node, resolveRef);
  const precedence = typePrecedence(node);
  return precedence === "function" ||
    (context === "intersection" && precedence === "union") ||
    (context === "union" && precedence === "intersection")
    ? `(${type})`
    : type;
}

function typeScriptType(
  node: TypeNode,
  resolveRef: (name: string) => string,
): string {
  const kind = node.kind as unknown;
  if (kind === "uuid" || kind === "json")
    return kind === "uuid" ? "string" : "unknown";
  const record = object(kind);
  if ("any" in record) return "SemanticValue";
  if ("unknown" in record) return "unknown";
  if ("never" in record) return "never";
  if ("null" in record) return "null";
  if ("bool" in record) return "boolean";
  if ("char" in record || "string" in record || "ip_addr" in record)
    return "string";
  if ("temporal" in record) return "number | bigint | Date";
  if ("number" in record) {
    const number = record.number;
    if (number && typeof number === "object") {
      if ("int" in number)
        return ["i8", "i16", "i24", "i32"].includes(String(number.int))
          ? "number"
          : "number | bigint";
      if ("uint" in number || "u_int" in number) {
        const width = number.uint ?? number.u_int;
        return ["u8", "u16", "u24", "u32"].includes(String(width))
          ? "number"
          : "number | bigint";
      }
      if (
        "float" in number ||
        "decimal" in number ||
        "rational" in number ||
        "complex" in number
      )
        return "number";
    }
    return "number | bigint";
  }
  if ("bytes" in record) return "Uint8Array";
  const optional = payload(kind, "optional");
  if (optional)
    return `${nestedType(optional.inner as TypeNode, resolveRef, "union")} | null`;
  const array = payload(kind, "array");
  if (array)
    return `Array<${typeScriptType(array.items as TypeNode, resolveRef)}>`;
  const list = payload(kind, "list");
  if (list)
    return `Array<${typeScriptType(list.items as TypeNode, resolveRef)}>`;
  const set = payload(kind, "set");
  if (set) return `Array<${typeScriptType(set.items as TypeNode, resolveRef)}>`;
  const tuple = payload(kind, "tuple");
  if (tuple) {
    const items = ((tuple.items as TypeNode[]) ?? []).map((item) =>
      typeScriptType(item, resolveRef),
    );
    if (tuple.rest)
      items.push(
        `...Array<${typeScriptType(tuple.rest as TypeNode, resolveRef)}>`,
      );
    return `[${items.join(", ")}]`;
  }
  const map = payload(kind, "map");
  if (map)
    return `Map<${typeScriptType(map.keys as TypeNode, resolveRef)}, ${typeScriptType(map.values as TypeNode, resolveRef)}>`;
  const recordType = payload(kind, "record");
  if (recordType) {
    const fields = Object.entries(recordType.fields ?? {}).map(
      ([name, raw]) => {
        const field = object(raw);
        return `${JSON.stringify(name)}${field.required ? "" : "?"}: ${typeScriptType(field.ty as TypeNode, resolveRef)}`;
      },
    );
    if (recordType.open) fields.push(`[key: string]: unknown`);
    return `{ ${fields.join("; ")} }`;
  }
  const attribute = payload(kind, "attribute");
  if (attribute) return typeScriptType(attribute.ty as TypeNode, resolveRef);
  const classType = payload(kind, "class");
  // A schema Class type is represented by the ID of an entity of that class.
  // The separately generated class declaration describes the entity object.
  if (classType) return "string";
  const union = payload(kind, "union");
  if (union)
    return (
      ((union.variants as TypeNode[]) ?? [])
        .map((item) => nestedType(item, resolveRef, "union"))
        .join(" | ") || "never"
    );
  const intersection = payload(kind, "intersection");
  if (intersection)
    return (
      ((intersection.variants as TypeNode[]) ?? [])
        .map((item) => nestedType(item, resolveRef, "intersection"))
        .join(" & ") || "unknown"
    );
  const result = payload(kind, "result");
  if (result)
    return `{ ok: ${typeScriptType(result.ok as TypeNode, resolveRef)} } | { err: ${typeScriptType(result.err as TypeNode, resolveRef)} }`;
  const enumType = payload(kind, "enum");
  if (enumType)
    return (
      ((enumType.variants as Array<Record<string, any>>) ?? [])
        .map((variant) =>
          literal(variant.symbol ?? variant.value ?? variant.name),
        )
        .join(" | ") || "never"
    );
  const variant = payload(kind, "variant");
  if (variant)
    return (
      ((variant.variants as Array<Record<string, any>>) ?? [])
        .map((item) => {
          const casePayload = object(item.payload);
          let value = "undefined";
          if ("tuple" in casePayload)
            value = `[${(casePayload.tuple as TypeNode[]).map((entry) => typeScriptType(entry, resolveRef)).join(", ")}]`;
          else if ("record" in casePayload)
            value = typeScriptType(
              {
                kind: { record: casePayload.record },
                constraints: [],
                annotations: [],
              },
              resolveRef,
            );
          else if ("newtype" in casePayload)
            value = typeScriptType(casePayload.newtype as TypeNode, resolveRef);
          return `{ $variant: ${JSON.stringify(item.name)}; value: ${value} }`;
        })
        .join(" | ") || "never"
    );
  const ref = payload(kind, "ref");
  if (ref) {
    const args = ((ref.args as TypeNode[]) ?? []).map((item) =>
      typeScriptType(item, resolveRef),
    );
    const resolved = resolveRef(String(ref.name));
    return `${resolved}${args.length && resolved !== "SemanticValue" && resolved !== "string" ? `<${args.join(", ")}>` : ""}`;
  }
  const stream = payload(kind, "stream");
  if (stream)
    return `AsyncIterable<${typeScriptType(stream.element as TypeNode, resolveRef)}>`;
  const handle = payload(kind, "handle");
  if (handle) {
    const reference = object(handle.interface);
    const args = ((reference.args as TypeNode[]) ?? []).map((item) =>
      typeScriptType(item, resolveRef),
    );
    return `${resolveRef(String(reference.name))}${args.length ? `<${args.join(", ")}>` : ""}`;
  }
  const interfaceType = payload(kind, "interface");
  if (interfaceType)
    return `{ ${((interfaceType.methods as Array<Record<string, any>>) ?? []).map((method) => `${JSON.stringify(String(method.name))}: ${functionType(object(method.signature), resolveRef)}`).join("; ")} }`;
  const fn = payload(kind, "function");
  if (fn) return functionType(fn, resolveRef);
  return "unknown";
}

function functionType(
  fn: Record<string, any>,
  resolveRef: (name: string) => string,
): string {
  const params = ((fn.params as Array<Record<string, any>>) ?? []).map(
    (param, index) =>
      `${identifier(String(param.name ?? `arg${index}`))}: ${typeScriptType(param.ty as TypeNode, resolveRef)}`,
  );
  const results = (fn.results as TypeNode[]) ?? [];
  const output =
    results.length === 0
      ? "void"
      : results.length === 1
        ? typeScriptType(results[0]!, resolveRef)
        : `[${results.map((item) => typeScriptType(item, resolveRef)).join(", ")}]`;
  return `(${params.join(", ")}) => ${fn.async_fn ? `Promise<${output}>` : output}`;
}

const contractCommands = (
  contract: Contract,
  prefix: string,
  resolveRef: (name: string) => string,
): Array<CommandModel & { prefix: string }> =>
  Object.values(contract.functions).map((fn) => ({
    name: fn.name,
    prefix,
    payload: fn.signature.params.map((param, index) => ({
      name: param.name ?? `arg${index}`,
      type: typeScriptType(param.ty, resolveRef),
      ...(isOptional(param.ty) ? { optional: true } : {}),
    })),
    output:
      fn.signature.results.length === 0
        ? "undefined"
        : fn.signature.results.length === 1
          ? typeScriptType(fn.signature.results[0]!, resolveRef)
          : `[${fn.signature.results.map((item) => typeScriptType(item, resolveRef)).join(", ")}]`,
    ...(typeof fn.meta.description === "string"
      ? { docs: fn.meta.description }
      : {}),
  }));

export function packageModel(pkg: Package): PackageModel {
  const modules: Array<[string, Module]> = [
    [pkg.root.name, pkg.root],
    ...Object.entries(pkg.modules),
  ];
  const candidates: Array<{
    rawName: string;
    aliases: string[];
    scope: string;
    node: TypeNode;
    generated?: string;
    params?: Array<{
      rawName: string;
      name: string;
      default: TypeNode | null;
    }>;
    docs?: string;
  }> = [];
  const addDeclarations = (
    scope: string,
    owner: {
      types: Module["types"];
      attributes: Module["attributes"];
      classes: Module["classes"];
      interfaces: Record<string, unknown>;
    },
  ): void => {
    for (const [key, definition] of Object.entries(owner.types))
      candidates.push({
        rawName: definition.name,
        aliases: [key, definition.name],
        scope,
        node: definition.ty,
        params: definition.params.map((param) => ({
          rawName: param.name,
          name: identifier(param.name),
          default: param.default,
        })),
        ...(definition.meta.description
          ? { docs: definition.meta.description }
          : {}),
      });
    for (const [key, attribute] of Object.entries(owner.attributes))
      candidates.push({
        rawName: attribute.name,
        aliases: [key, attribute.id, attribute.name],
        scope,
        node: attribute.ty,
        ...(attribute.meta.description
          ? { docs: attribute.meta.description }
          : {}),
      });
    for (const [key, interfaceType] of Object.entries(owner.interfaces)) {
      const contractInterface = interfaceType as unknown as Record<string, any>;
      const wrapped = "interface" in contractInterface;
      candidates.push({
        rawName: wrapped ? String(contractInterface.name ?? key) : key,
        aliases: wrapped ? [key, String(contractInterface.name ?? key)] : [key],
        scope,
        node: {
          kind: {
            interface: wrapped
              ? contractInterface.interface
              : contractInterface,
          },
          constraints: [],
          annotations: [],
        },
        ...(wrapped && contractInterface.meta?.description
          ? { docs: String(contractInterface.meta.description) }
          : {}),
      });
    }
  };
  for (const [moduleName, module] of modules) {
    addDeclarations(moduleName, module);
    for (const [contractName, contract] of Object.entries(module.contracts))
      addDeclarations(`${moduleName}::${contractName}`, contract);
  }
  const counts = new Map<string, number>();
  for (const candidate of candidates)
    counts.set(
      identifier(candidate.rawName),
      (counts.get(identifier(candidate.rawName)) ?? 0) + 1,
    );
  const qualifiedNames = new Map<string, string>();
  const unqualifiedNames = new Map<string, Set<string>>();
  const allocated = new Set<string>();
  for (const candidate of candidates) {
    const simple = identifier(candidate.rawName);
    const base =
      counts.get(simple) === 1
        ? simple
        : `${candidate.scope.split("::").map(identifier).join("_")}_${simple}`;
    let generated = base;
    for (let suffix = 2; allocated.has(generated); suffix++)
      generated = `${base}_${suffix}`;
    allocated.add(generated);
    candidate.generated = generated;
    for (const alias of candidate.aliases) {
      qualifiedNames.set(`${candidate.scope}::${alias}`, generated);
      const matches = unqualifiedNames.get(alias) ?? new Set<string>();
      matches.add(generated);
      unqualifiedNames.set(alias, matches);
    }
  }
  const usedTypeNames = new Set(allocated);
  const classQualifiedNames = new Map<string, string>();
  const classUnqualifiedNames = new Map<string, Set<string>>();
  const classPlans: Array<{
    scope: string;
    key: string;
    value: ClassType;
    generated: string;
  }> = [];
  for (const [moduleName, module] of modules) {
    const owners: Array<[string, Record<string, ClassType>]> = [
      [moduleName, module.classes],
      ...Object.entries(module.contracts).map(
        ([contractName, contract]) =>
          [`${moduleName}::${contractName}`, contract.classes] as [
            string,
            Record<string, ClassType>,
          ],
      ),
    ];
    for (const [scope, classes] of owners)
      for (const [name, klass] of Object.entries(classes)) {
        const value = klass as ClassType;
        const simple = identifier(value.name || name);
        const base = usedTypeNames.has(simple)
          ? `${scope.split("::").map(identifier).join("_")}_${simple}`
          : simple;
        let generated = base;
        for (let suffix = 2; usedTypeNames.has(generated); suffix++)
          generated = `${base}_${suffix}`;
        usedTypeNames.add(generated);
        classPlans.push({ scope, key: name, value, generated });
        for (const alias of [name, value.id, value.name]) {
          classQualifiedNames.set(`${scope}::${alias}`, generated);
          const matches = classUnqualifiedNames.get(alias) ?? new Set<string>();
          matches.add(generated);
          classUnqualifiedNames.set(alias, matches);
        }
      }
  }
  const resolveRef = (name: string, scope: string): string => {
    const normalized = name.replaceAll("/", "::").replace(/^::/, "");
    if (normalized.includes("::")) {
      const direct = qualifiedNames.get(normalized);
      if (direct) return direct;
      const moduleScope = scope.split("::")[0]!;
      const relative = qualifiedNames.get(`${moduleScope}::${normalized}`);
      if (relative) return relative;
    } else {
      const scopes = scope.split("::");
      while (scopes.length) {
        const local = qualifiedNames.get(`${scopes.join("::")}::${normalized}`);
        if (local) return local;
        scopes.pop();
      }
      const matches = unqualifiedNames.get(normalized);
      if (matches?.size === 1) return [...matches][0]!;
      if (matches && matches.size > 1)
        throw new Error(
          `ambiguous type reference '${name}' from '${scope}'; use a qualified reference`,
        );
    }
    // References to class IDs are entity references on the wire, not embedded
    // class-shaped objects.
    if (normalized.includes("::")) {
      if (classQualifiedNames.has(normalized)) return "string";
      const moduleScope = scope.split("::")[0]!;
      if (classQualifiedNames.has(`${moduleScope}::${normalized}`))
        return "string";
    }
    const classScopes = scope.split("::");
    while (classScopes.length) {
      if (classQualifiedNames.has(`${classScopes.join("::")}::${normalized}`))
        return "string";
      classScopes.pop();
    }
    const classes = classUnqualifiedNames.get(normalized);
    if (classes?.size === 1) return "string";
    if (classes && classes.size > 1)
      throw new Error(
        `ambiguous class reference '${name}' from '${scope}'; use a qualified reference`,
      );
    return ["id", "string", "uuid"].includes(name.toLowerCase())
      ? "string"
      : "SemanticValue";
  };
  const resolveClassRef = (name: string, scope: string): string => {
    const normalized = name.replaceAll("/", "::").replace(/^::/, "");
    if (normalized.includes("::")) {
      const direct = classQualifiedNames.get(normalized);
      if (direct) return direct;
      const moduleScope = scope.split("::")[0]!;
      const relative = classQualifiedNames.get(`${moduleScope}::${normalized}`);
      if (relative) return relative;
    } else {
      const scopes = scope.split("::");
      while (scopes.length) {
        const local = classQualifiedNames.get(
          `${scopes.join("::")}::${normalized}`,
        );
        if (local) return local;
        scopes.pop();
      }
      const matches = classUnqualifiedNames.get(normalized);
      if (matches?.size === 1) return [...matches][0]!;
      if (matches && matches.size > 1)
        throw new Error(
          `ambiguous class reference '${name}' from '${scope}'; use a qualified reference`,
        );
    }
    // External base classes are not available for structural expansion.
    return "SemanticObject";
  };
  const types: TypeModel[] = candidates.map((candidate) => {
    const scopedResolve = (name: string): string =>
      candidate.params?.find(
        (param) => param.rawName === name || param.name === identifier(name),
      )?.name ?? resolveRef(name, candidate.scope);
    return {
      name: candidate.generated!,
      type: typeScriptType(candidate.node, scopedResolve),
      ...(candidate.params?.length
        ? {
            params: candidate.params.map((param) => ({
              name: param.name,
              ...(param.default === null
                ? {}
                : { default: typeScriptType(param.default, scopedResolve) }),
            })),
          }
        : {}),
      ...(candidate.docs ? { docs: candidate.docs } : {}),
    };
  });
  for (const { value, generated, scope } of classPlans) {
    const fields = Object.entries(value.attributes).map(
      ([fieldName, field]) =>
        `${JSON.stringify(fieldName)}${field.required ? "" : "?"}: ${resolveRef(field.attribute.id, scope)}`,
    );
    const inherited = [
      ...new Set(
        [value.inherits, ...value.extends]
          .filter((item): item is { id: string } => item !== null)
          .map((item) => resolveClassRef(item.id, scope)),
      ),
    ];
    if (
      !value["semantic:class:strict_schema"] &&
      !inherited.includes("SemanticObject")
    )
      inherited.push("SemanticObject");
    types.push({
      name: generated,
      type: [`{ ${fields.join("; ")} }`, ...inherited]
        .map((item) => `(${item})`)
        .join(" & "),
      ...(value.meta.description ? { docs: value.meta.description } : {}),
    });
  }
  const commandCandidates = modules.flatMap(([moduleName, module]) =>
    Object.entries(module.contracts).flatMap(([contractName, contract]) =>
      contractCommands(contract, `${moduleName}_${contractName}`, (name) =>
        resolveRef(name, `${moduleName}::${contractName}`),
      ),
    ),
  );
  const commandCounts = new Map<string, number>();
  for (const command of commandCandidates)
    commandCounts.set(
      identifier(command.name),
      (commandCounts.get(identifier(command.name)) ?? 0) + 1,
    );
  const commandSymbols = new Set<string>(usedTypeNames);
  const commands = commandCandidates.map(({ prefix, ...command }) => {
    const simple = identifier(command.name);
    const base =
      commandCounts.get(simple) === 1
        ? simple
        : `${identifier(prefix)}_${simple}`;
    let symbol = base;
    for (let suffix = 2; commandSymbols.has(symbol); suffix++)
      symbol = `${base}_${suffix}`;
    commandSymbols.add(symbol);
    return { ...command, symbol };
  });
  return { name: pkg.name, types, commands };
}
