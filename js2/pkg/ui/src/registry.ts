import {
  EntityType,
  genericEntityTitle,
  FACTOR_ENTITY_ATTRIBUTES,
  FACTOR_EXTEND,
  FACTOR_IDENT,
  FACTOR_TYPE,
} from '@semantic/api';
import { UiPlugin } from './plugin';
import {
  Attribute,
  Cardinality,
  Class,
  SemanticSchema,
} from '@semantic/api/src/core';
import { AttrName } from '@semantic/api/src/db';
import { renderGenericEntityBox } from './component/entity/generic';
import { ReactNode } from 'react';

// import { renderGenericEntityBox } from "../component/entity";
// import { UiPlugin } from "./plugin";
// import { FieldAccessor, FormValidator } from "../component/form";

export type ValueMap = Record<string, any>;

export interface EntityRenderOpts {
  preview: boolean;
  allowDelete?: boolean;
  allowEdit?: boolean;
  onDeleted?: (item: ValueMap) => void;
  onModified?: (item: ValueMap) => void;
}

export type EntityTitleRenderer = (entity: ValueMap) => string;

export type EntityContentRenderer = (
  item: ValueMap,
  opts: EntityRenderOpts,
) => ReactNode;

export type AttributeRenderer = (value: any, item: ValueMap) => ReactNode;

export type EntityRenderer = (
  item: ValueMap,
  opts: EntityRenderOpts,
) => ReactNode;

export type EditableEntityRenderer = (
  entity: ValueMap,
  opts: EntityRenderOpts,
  onChanged?: (newItem: ValueMap) => void,
) => ReactNode;

export interface MediaHandle {
  play(): void;
  pause(): void;
  mute(): void;
  unmute(): void;
  isPlaying(): boolean;
  isFinished(): boolean;
  // Progress as a float between 0 and 1.
  progress(): number | null;
}

export interface MediaRenderProps {
  item: ValueMap;

  autoStart: boolean;
  showControls: boolean;

  onPaused(): void;
  onResumed(): void;
  onFinished(): void;
  onFailed(error: string): void;
  onProgress(progress: number): void;
  onDurationAvailable(durationSeconds: Number): void;
}

export type EntityMediaRenderer = (
  props: MediaRenderProps,
) => [MediaHandle | null, ReactNode];

export interface AttributeFieldRendererProps {
  // field: FieldAccessor<any>;
  field: any;
  attribute: Attribute;
  cardinality: Cardinality;
}

export type AttributeFieldRenderer = (
  props: AttributeFieldRendererProps,
) => [ReactNode, any];

export type ClassMap<T> = Record<EntityType, T>;

export class UiRegistry {
  schema: SemanticSchema;

  attrs: Record<AttrName, Attribute> = {};
  classes: ClassMap<Class> = {};
  classParentMap: ClassMap<Set<EntityType>>;
  hiddenClasses: Set<EntityType> = new Set();

  plugins: Record<string, UiPlugin> = {};

  attributeRenderers: Record<AttrName, AttributeRenderer> = {};
  private attributeFieldrenderers: Record<AttrName, AttributeFieldRenderer> =
    {};

  entityTitleRenderers: ClassMap<EntityTitleRenderer> = {};
  entityContentRenderers: ClassMap<EntityContentRenderer> = {};
  entityMediaRenderers: ClassMap<EntityMediaRenderer> = {};
  entityRenderers: ClassMap<EntityRenderer> = {};
  editableEntityRenderers: ClassMap<EditableEntityRenderer> = {};

  constructor(schema: SemanticSchema) {
    this.schema = schema;

    for (const attr of schema.db.attributes) {
      this.attrs[attr[FACTOR_IDENT]] = attr;
    }

    for (const cls of schema.db.classes) {
      const ident = cls[FACTOR_IDENT];
      this.classes[ident] = cls;
      this.entityTitleRenderers[ident] = genericEntityTitle;
      // this.entityContentRenderers[ident] = tableRender;
    }

    // Build parents map.
    const map: Record<EntityType, Set<EntityType>> = {};
    for (const cls of schema.db.classes) {
      let parents: EntityType[] = cls[FACTOR_EXTEND] ?? [];
      const set = new Set<EntityType>([]);

      while (true) {
        const parentIdent = parents.pop();
        if (!parentIdent) {
          break;
        }
        set.add(parentIdent);
        const nestedParents = this.classes[parentIdent][FACTOR_EXTEND] ?? [];
        parents = [...parents, ...nestedParents];
      }
      map[cls[FACTOR_IDENT]] = set;
    }
    this.classParentMap = map;
  }

  registerPlugin(plugin: UiPlugin) {
    this.plugins[plugin.name()] = plugin;
    const schema = plugin.schema();

    for (const [attr, render] of Object.entries(
      schema.attributeRenderers ?? {},
    )) {
      this.attributeRenderers[attr] = render;
    }
    for (const [attr, render] of Object.entries(
      schema.attributeFieldRenderers ?? {},
    )) {
      this.attributeFieldrenderers[attr] = render;
    }

    for (const [ty, render] of Object.entries(
      schema.entityTitleRenderers ?? {},
    )) {
      this.entityTitleRenderers[ty] = render;
    }
    for (const [ty, render] of Object.entries(
      schema.entityContentRenderers ?? {},
    )) {
      this.entityContentRenderers[ty] = render;
    }
    for (const [ty, render] of Object.entries(
      schema.entityMediaRenderers ?? {},
    )) {
      this.entityMediaRenderers[ty] = render;
    }
    for (const [ty, render] of Object.entries(schema.entityRenderers ?? {})) {
      this.entityRenderers[ty] = render;
    }
    for (const [ty, render] of Object.entries(
      schema.editableEntityRenderers ?? {},
    )) {
      this.editableEntityRenderers[ty] = render;
    }
  }

  getEntityType(ty: EntityType): Class | null {
    return this.classes[ty] ?? null;
  }

  mustGetEntityType(ty: EntityType): Class {
    const entity = this.classes[ty];
    if (!entity) {
      throw new Error(`Entity type ${ty} not found`);
    }
    return entity;
  }

  mustGetAttribute(attr: AttrName): Attribute {
    const attrSchema = this.attrs[attr];
    if (!attrSchema) {
      throw new Error(`Attribute ${attr} not found`);
    }
    return attrSchema;
  }

  attributeFieldRenderer(ident: AttrName): AttributeFieldRenderer | null {
    return this.attributeFieldrenderers[ident] ?? null;
  }

  entityTitle(entity: ValueMap): string {
    const ty: string = entity[FACTOR_TYPE];
    return (
      this.entityTitleRenderers[ty]?.(entity) || genericEntityTitle(entity)
    );
  }

  entityAttributes(ty: EntityType): [Attribute, Cardinality][] {
    const schema = this.mustGetEntityType(ty);
    return schema[FACTOR_ENTITY_ATTRIBUTES].map((field) => {
      const as = this.mustGetAttribute(field['factor/attribute']);
      return [as, field['factor/required'] ? 'Required' : 'Optional'];
    });
  }

  renderEntity(item: ValueMap, opts: EntityRenderOpts): ReactNode {
    const ty = item[FACTOR_TYPE];
    const render = this.entityRenderers[ty];
    if (render) {
      return render(item, opts);
    } else {
      return renderGenericEntityBox(this, item, opts);
    }
  }

  mediaRenderer(ty: EntityType): EntityMediaRenderer | null {
    return this.entityMediaRenderers[ty] ?? null;
  }

  renderEntityMedia(
    props: MediaRenderProps,
  ): [MediaHandle | null, ReactNode] | null {
    const ty = props.item[FACTOR_TYPE];
    const render = this.entityMediaRenderers[ty];
    return render ? render(props) : null;
  }

  renderEditableEntity(
    item: ValueMap,
    opts: EntityRenderOpts,
    onChanged?: (newItem: ValueMap) => void,
  ): ReactNode {
    const ty = item[FACTOR_TYPE];
    const render = this.editableEntityRenderers[ty];
    if (render) {
      return render(item, opts, onChanged);
    } else {
      return renderGenericEntityBox(this, item, opts);
    }
  }
}
