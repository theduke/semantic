import { FACTOR_ID, FACTOR_IDENT, SEMANTIC_PREVIEW_IMAGE_URL, SEMANTIC_TAG_NAME, SEMANTIC_TITLE, SEMANTIC_URL, TY_SEMANTIC_AUDIO, TY_SEMANTIC_IMAGE, TY_SEMANTIC_TAG, TY_SEMANTIC_VIDEO, ValueMap } from "@semantic/api";
import { PluginSchema, UiPlugin } from "../plugin";
import { ReactNode } from "react";
import { renderImage } from "./image";
import { renderVideo } from "./video";
import { renderAudio } from "./audio";

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
        attributeRenderers: {
          // [SEMANTIC_NOTE_BODY]: renderMarkdownAttr,
          // [SEMANTIC_MARKDOWN_BODY]: renderMarkdownAttr,
          [SEMANTIC_PREVIEW_IMAGE_URL]: previewImageUrlAttrRenderer,
        },
        attributeFieldRenderers: {
          // [SEMANTIC_NOTE_BODY]: renderAttrFieldTextArea,
          // [SEMANTIC_MARKDOWN_BODY]: renderAttrFieldTextArea,
        },
        entityContentRenderers: {
          [TY_SEMANTIC_IMAGE]: renderImage,
          [TY_SEMANTIC_VIDEO]: renderVideo,
          [TY_SEMANTIC_AUDIO]: renderAudio,
        },
        entityMediaRenderers: {
          // [TY_SEMANTIC_VIDEO]: renderVideoMedia,
          // [TY_SEMANTIC_IMAGE]: renderImageMedia,
          // [TY_SEMANTIC_AUDIO]: renderAudioMedia,
        },
        entityTitleRenderers: {
          [TY_SEMANTIC_TAG]: tagTitleRenderer,
        },
      };
    },
  };
}

function tagTitleRenderer(tag: ValueMap) {
  return tag[SEMANTIC_TAG_NAME] ||
    tag[SEMANTIC_TITLE] ||
    tag[FACTOR_IDENT] ||
    tag[FACTOR_ID] ||
    "<no title>";
}

function previewImageUrlAttrRenderer(
  value: any,
  item: ValueMap
): ReactNode {
  const img = (
    <img
      src={value}
      style={{ maxWidth: "100%", maxHeight: "300px" }
      }
    />
  );
  const url = item[SEMANTIC_URL];

  if (url) {
    return <a href={url}> {img} </a>;
  } else {
    return img;
  }
}
