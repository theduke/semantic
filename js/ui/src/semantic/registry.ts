import { JSX } from "solid-js/jsx-runtime";
import { AttributeName, EntityType, genericEntityTitle } from ".";
import { renderAttrValue, renderGenericEntityBox } from "../component/entity";
import {
  AttributeSchema,
  Cardinality,
  EntitySchema,
  SemanticSchema,
} from "semantic/dist/core";
import { UiPlugin } from "./plugin";
import { FACTOR_ENTITY_ATTRIBUTES, FACTOR_IDENT, FACTOR_TYPE } from "semantic/dist/schema";

export type ValueMap = Record<string, any>;

export interface EntityRenderOpts {
  preview: boolean;
  allowDelete?: boolean;
  allowEdit?: boolean;
  onDeleted?: (item: ValueMap) => void;
  onModified?: (item: ValueMap) => void;
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
  onChanged?: (newItem: ValueMap) => void
) => JSX.Element;

export interface MediaHandle {
  play(): void;
  pause(): void;
  mute(): void;
  unmute(): void;
  isPlaying(): boolean;
  // Progress as a float between 0 and 1.
  progress(): number;
}

export interface MediaRenderProps {
  item: ValueMap;
  onPaused(): void;
  onResumed(): void;
  onFinished(): void;
  onFailed(error: string): void;
}

export type EntityMediaRenderer = (
  props: MediaRenderProps
) => [MediaHandle, JSX.Element];

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
  entityMediaRenderers: EntityTypeMap<EntityMediaRenderer> = {};
  entityRenderers: EntityTypeMap<EntityRenderer> = {};
  editableEntityRenderers: EntityTypeMap<EditableEntityRenderer> = {};

  constructor(schema: SemanticSchema) {
    this.schema = schema;

    // const tableRender = (item: ValueMap) => renderEntityTable(this, item);
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
      // this.entityContentRenderers[ident] = tableRender;
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
    for (const [ty, render] of Object.entries(
      schema.entityMediaRenderers ?? {}
    )) {
      this.entityMediaRenderers[ty] = render;
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

  getEntityType(ty: EntityType): EntitySchema | null {
    return this.entityTypes[ty] ?? null;
  }

  mustGetEntityType(ty: EntityType): EntitySchema {
    const entity = this.entityTypes[ty];
    if (!entity) {
      throw new Error(`Entity type ${ty} not found`);
    }
    return entity;
  }

  mustGetAttribute(attr: AttributeName): AttributeSchema {
    const attrSchema = this.attrs[attr];
    if (!attrSchema) {
      throw new Error(`Attribute ${attr} not found`);
    }
    return attrSchema;
  }

  entityTitle(entity: ValueMap): string {
    return genericEntityTitle(entity);
  }

  entityAttributes(ty: EntityType): [AttributeSchema, Cardinality][] {
    const schema = this.mustGetEntityType(ty);
    return schema[FACTOR_ENTITY_ATTRIBUTES].map((field) => {
      const as = this.mustGetAttribute(field.attribute);
      return [as, field.cardinality];
    });
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

  renderEntityMedia(
    props: MediaRenderProps
  ): [MediaHandle, JSX.Element] | null {
    const ty = props.item[FACTOR_TYPE];
    const render = this.entityMediaRenderers[ty];
    return render ? render(props) : null;
  }

  renderEditableEntity(
    item: ValueMap,
    opts: EntityRenderOpts,
    onChanged?: (newItem: ValueMap) => void
  ): JSX.Element {
    const ty = item[FACTOR_TYPE];
    const render = this.editableEntityRenderers[ty];
    if (render) {
      return render(item, opts, onChanged);
    } else {
      return renderGenericEntityBox(this, item, opts);
    }
  }
}
