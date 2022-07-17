import { JSX } from "solid-js/jsx-runtime";
import { AttributeName, EntityType, genericEntityTitle } from ".";
import {
  renderAttrValue,
  renderEntityTable,
  renderGenericEntityBox,
} from "../component/entity";
import { AttributeSchema, EntitySchema, SemanticSchema } from "./core";
import { UiPlugin } from "./plugin";
import { FACTOR_IDENT, FACTOR_TYPE } from "./schema";

export type ValueMap = Record<string, any>;

export interface EntityRenderOpts {
  preview: boolean;
}

export type EntityTitleRenderer = (entity: ValueMap) => JSX.Element;
export type EntityContentRenderer = (
  item: ValueMap,
  opts: EntityRenderOpts
) => JSX.Element;
export type AttributeRenderer = (value: any, item: ValueMap) => JSX.Element;
export type EntityRenderer = (
  item: ValueMap,
  opts: EntityRenderOpts
) => JSX.Element;
export type EditableEntityRenderer = (
  entity: ValueMap,
  opts: EntityRenderOpts,
  onChanged: (newItem: ValueMap) => void
) => JSX.Element;

export type EntityTypeMap<T> = Record<EntityType, T>;

export class UiRegistry {
  schema: SemanticSchema;

  attrs: Record<AttributeName, AttributeSchema> = {};
  entityTypes: EntityTypeMap<EntitySchema> = {};

  hiddenEntityTypes: Set<EntityType> = new Set();

  plugins: Record<string, UiPlugin> = {};

  attributeRenderers: Record<AttributeName, AttributeRenderer> = {};
  entityTitleRenderers: EntityTypeMap<EntityTitleRenderer> = {};
  entityContentRenderers: EntityTypeMap<EntityContentRenderer> = {};
  entityRenderers: EntityTypeMap<EntityRenderer> = {};
  editableEntityRenderers: EntityTypeMap<EditableEntityRenderer> = {};

  constructor(schema: SemanticSchema) {
    this.schema = schema;

    const tableRender = (item: ValueMap) => renderEntityTable(this, item);
    const attrRenderer = (attr: AttributeName, value: any) =>
      renderAttrValue(this, attr, value);

    for (const attr of schema.db.attributes) {
      const ident = attr[FACTOR_IDENT];
      this.attrs[ident] = attr;
      this.attributeRenderers[ident] = attrRenderer;
    }

    for (const entity of schema.db.entities) {
      const ident = entity[FACTOR_IDENT];
      this.entityTypes[ident] = entity;
      this.entityTitleRenderers[ident] = genericEntityTitle;
      this.entityContentRenderers[ident] = tableRender;
    }
  }

  registerPlugin(plugin: UiPlugin) {
    this.plugins[plugin.name()] = plugin;
    const schema = plugin.schema();

    for (const [attr, render] of Object.entries(
      schema.attributeRenderers ?? {}
    )) {
      this.attributeRenderers[attr] = render;
    }
    for (const [ty, render] of Object.entries(
      schema.entityTitleRenderers ?? {}
    )) {
      this.entityTitleRenderers[ty] = render;
    }
    for (const [ty, render] of Object.entries(
      schema.entityContentRenderers ?? {}
    )) {
      this.entityContentRenderers[ty] = render;
    }
    for (const [ty, render] of Object.entries(schema.entityRenderers ?? {})) {
      this.entityRenderers[ty] = render;
    }
    for (const [ty, render] of Object.entries(
      schema.editableEntityRenderers ?? {}
    )) {
      this.editableEntityRenderers[ty] = render;
    }
  }

  entityTitle(entity: ValueMap): string {
    return genericEntityTitle(entity);
  }

  renderEntity(item: ValueMap, opts: EntityRenderOpts): JSX.Element {
    const ty = item[FACTOR_TYPE];
    const render = this.entityRenderers[ty];
    if (render) {
      return render(item, opts);
    } else {
      return renderGenericEntityBox(this, item, opts);
    }
  }

  renderEditableEntity(
    item: ValueMap,
    opts: EntityRenderOpts,
    onChanged: (newItem: ValueMap) => void
  ): JSX.Element {
    const ty = item.data[FACTOR_TYPE];
    const render = this.editableEntityRenderers[ty];
    if (render) {
      return render(item, opts, onChanged);
    } else {
      return renderGenericEntityBox(this, item, opts);
    }
  }
}
