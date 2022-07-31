import { PluginSchema, UiPlugin } from "../plugin";
import { TY_SEMANTIC_IMAGE, TY_SEMANTIC_VIDEO } from "semantic/dist/schema";
import { renderImage } from "./image";
import { renderVideo } from "./video";

export function basePlugin(): UiPlugin {
  return {
    name(): string {
      return "semantic/Base";
    },
    version(): string {
      return "0.0.1";
    },
    schema(): PluginSchema {
      return {
        entityContentRenderers: {
          [TY_SEMANTIC_IMAGE]: renderImage,
          [TY_SEMANTIC_VIDEO]: renderVideo,
        },
      };
    },
  };
}
