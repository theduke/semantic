import type {
  BatchOperation,
  Contract,
  ContractFunction,
  Meta,
  Migration,
  MigrationOperation,
  Module,
  Package,
  SchemaVersion,
  SemanticObject,
  TypeDef,
} from "./types.js";
const dictionary = <T>(): Record<string, T> =>
  Object.create(null) as Record<string, T>;
const clone = <T>(value: T): T => structuredClone(value);
export const emptyMeta = (): Meta => ({
  title: null,
  description: null,
  id: null,
  deprecated: null,
  aliases: [],
  examples: [],
  tags: [],
  docs_url: null,
  annotations: dictionary(),
});
const emptyModule = (name: string): Module => ({
  name,
  constants: dictionary(),
  types: dictionary(),
  attributes: dictionary(),
  classes: dictionary(),
  interfaces: dictionary(),
  contracts: dictionary(),
  meta: emptyMeta(),
});
export class PackageBuilder {
  private readonly value: Package;
  constructor(name: string, rootModule = "root") {
    this.value = {
      name,
      root: emptyModule(rootModule),
      modules: dictionary(),
      migrations: [],
      version: null,
      meta: emptyMeta(),
    };
  }
  version(version: SchemaVersion): this {
    this.value.version = clone(version);
    return this;
  }
  meta(meta: Partial<Meta>): this {
    this.value.meta = { ...this.value.meta, ...clone(meta) };
    return this;
  }
  root(configure: (module: ModuleBuilder) => void): this {
    configure(new ModuleBuilder(this.value.root));
    return this;
  }
  module(name: string, configure?: (module: ModuleBuilder) => void): this {
    const module = this.value.modules[name] ?? emptyModule(name);
    this.value.modules[name] = module;
    configure?.(new ModuleBuilder(module));
    return this;
  }
  migration(
    module: string,
    name: string,
    configure: (migration: MigrationBuilder) => void,
  ): this {
    const migration: Migration = {
      module,
      name,
      description: null,
      operations: [],
      meta: emptyMeta(),
    };
    configure(new MigrationBuilder(migration));
    this.value.migrations.push(migration);
    return this;
  }
  build(): Package {
    return clone(this.value);
  }
}
export class ModuleBuilder {
  constructor(private readonly value: Module) {}
  type(definition: TypeDef): this {
    if (this.value.types[definition.name])
      throw new Error(`duplicate type '${definition.name}'`);
    this.value.types[definition.name] = clone(definition);
    return this;
  }
  attribute(name: string, definition: Module["attributes"][string]): this {
    if (this.value.attributes[name])
      throw new Error(`duplicate attribute '${name}'`);
    this.value.attributes[name] = clone(definition);
    return this;
  }
  class(name: string, definition: Module["classes"][string]): this {
    if (this.value.classes[name]) throw new Error(`duplicate class '${name}'`);
    this.value.classes[name] = clone(definition);
    return this;
  }
  contract(
    name: string,
    configure?: (contract: ContractBuilder) => void,
  ): this {
    const contract: Contract = this.value.contracts[name] ?? {
      name,
      constants: dictionary(),
      types: dictionary(),
      functions: dictionary(),
      attributes: dictionary(),
      classes: dictionary(),
      interfaces: dictionary(),
      meta: emptyMeta(),
    };
    this.value.contracts[name] = contract;
    configure?.(new ContractBuilder(contract));
    return this;
  }
  meta(meta: Partial<Meta>): this {
    this.value.meta = { ...this.value.meta, ...clone(meta) };
    return this;
  }
}
export class ContractBuilder {
  constructor(private readonly value: Contract) {}
  command(
    name: string,
    signature: ContractFunction["signature"],
    meta: Meta = emptyMeta(),
  ): this {
    if (this.value.functions[name])
      throw new Error(`duplicate command '${name}'`);
    this.value.functions[name] = clone({ name, signature, meta });
    return this;
  }
  build(): Contract {
    return clone(this.value);
  }
}
export class MigrationBuilder {
  constructor(private readonly value: Migration) {}
  description(description: string): this {
    this.value.description = description;
    return this;
  }
  operation(operation: MigrationOperation): this {
    this.value.operations.push(clone(operation));
    return this;
  }
  insert(collection: string, id: string, object: SemanticObject): this {
    return this.operation({ insert: { collection, id, object } });
  }
}
export class BatchBuilder {
  private readonly operations: BatchOperation[] = [];
  upsert(id: string, object: SemanticObject, collection = "default"): this {
    this.operations.push({
      kind: "upsert",
      collection,
      id,
      object: clone(object),
    });
    return this;
  }
  delete(id: string, collection = "default"): this {
    this.operations.push({ kind: "delete_by_id", collection, id });
    return this;
  }
  deleteMany(ids: string[], collection = "default"): this {
    this.operations.push({
      kind: "delete_by_ids",
      collection,
      ids: clone(ids),
    });
    return this;
  }
  build(): BatchOperation[] {
    return clone(this.operations);
  }
}
