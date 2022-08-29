import { AttributeName } from ".";
import {
  AttributeRenderer,
  EditableEntityRenderer,
  EntityContentRenderer,
  EntityMediaRenderer,
  EntityRenderer,
  EntityTitleRenderer,
  ClassMap,
} from "./registry";

export type SemanticVersion = string;

export interface PluginSchema {
  attributeRenderers?: Record<AttributeName, AttributeRenderer>;
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
