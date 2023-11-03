import { AttrName } from '@semantic/api/src/db';
import {
  AttributeRenderer,
  EditableEntityRenderer,
  EntityContentRenderer,
  EntityMediaRenderer,
  EntityRenderer,
  EntityTitleRenderer,
  ClassMap,
  AttributeFieldRenderer,
} from './registry';

export type SemanticVersion = string;

export interface PluginSchema {
  attributeRenderers?: Record<AttrName, AttributeRenderer>;
  attributeFieldRenderers?: Record<AttrName, AttributeFieldRenderer>;

  entityTitleRenderers?: ClassMap<EntityTitleRenderer>;
  entityContentRenderers?: ClassMap<EntityContentRenderer>;
  entityMediaRenderers?: ClassMap<EntityMediaRenderer>;
  entityRenderers?: ClassMap<EntityRenderer>;
  editableEntityRenderers?: ClassMap<EditableEntityRenderer>;
}

export interface UiPlugin {
  name(): string;
  version(): SemanticVersion;
  schema(): PluginSchema;
}
