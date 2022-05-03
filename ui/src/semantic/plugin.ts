import { AttributeName } from ".";
import {
  AttributeRenderer,
  EditableEntityRenderer,
  EntityContentRenderer,
  EntityRenderer,
  EntityTitleRenderer,
  EntityTypeMap,
} from "./registry";

export type SemanticVersion = string;

export interface PluginSchema {
  attributeRenderers?: Record<AttributeName, AttributeRenderer>;
  entityTitleRenderers?: EntityTypeMap<EntityTitleRenderer>;
  entityContentRenderers?: EntityTypeMap<EntityContentRenderer>;
  entityRenderers?: EntityTypeMap<EntityRenderer>;
  editableEntityRenderers?: EntityTypeMap<EditableEntityRenderer>;
}

export interface UiPlugin {
  name(): string;
  version(): SemanticVersion;
  schema(): PluginSchema;
}
