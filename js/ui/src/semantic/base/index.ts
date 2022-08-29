import { PluginSchema, UiPlugin } from "../plugin";
import {
  FACTOR_ID,
  FACTOR_IDENT,
  SEMANTIC_TAG_NAME,
  SEMANTIC_TITLE,
  TY_SEMANTIC_AUDIO,
  TY_SEMANTIC_IMAGE,
  TY_SEMANTIC_TAG,
  TY_SEMANTIC_VIDEO,
} from "semantic/dist/schema";
import { renderImage, renderImageMedia } from "./image";
import { renderVideo, renderVideoMedia } from "./video";
import { renderAudio, renderAudioMedia } from "./audio";

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
          [TY_SEMANTIC_AUDIO]: renderAudio,
        },
        entityMediaRenderers: {
          [TY_SEMANTIC_VIDEO]: renderVideoMedia,
          [TY_SEMANTIC_IMAGE]: renderImageMedia,
          [TY_SEMANTIC_AUDIO]: renderAudioMedia,
        },
        entityTitleRenderers: {
          [TY_SEMANTIC_TAG]: (tag) => tag[SEMANTIC_TAG_NAME] || tag[SEMANTIC_TITLE] || tag[FACTOR_IDENT] || tag[FACTOR_ID] || '???',
        }
      };
    },
  };
}
