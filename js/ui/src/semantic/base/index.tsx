import { PluginSchema, UiPlugin } from "../plugin";
import {
  FACTOR_ID,
  FACTOR_IDENT,
  SEMANTIC_NOTE_BODY,
  SEMANTIC_PREVIEW_IMAGE_URL,
  SEMANTIC_TAG_NAME,
  SEMANTIC_TEXT_FORMAT,
  SEMANTIC_TITLE,
  SEMANTIC_URL,
  TY_SEMANTIC_AUDIO,
  TY_SEMANTIC_IMAGE,
  TY_SEMANTIC_TAG,
  TY_SEMANTIC_VIDEO,
} from "semantic/dist/schema";
import { renderImage, renderImageMedia } from "./image";
import { renderVideo, renderVideoMedia } from "./video";
import { renderAudio, renderAudioMedia } from "./audio";
import { renderAttrFieldTextArea } from "../../component/entity/entity_form";
import { ValueMap } from "../registry";
import { JSX } from "solid-js";
import SolidMarkdown from "solid-markdown";

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
          [SEMANTIC_NOTE_BODY]: (value: any, item: ValueMap): JSX.Element => {
            const content = typeof value === "string" ? value.trim() : "";

            if (content === "") {
              return null;
            }

            const rawFormat = item[SEMANTIC_TEXT_FORMAT];
            const format = rawFormat === "markdown" ? "markdown" : null;
            console.debug({ item, format, rawFormat });

            if (format === "markdown") {
              return <SolidMarkdown children={content} />;
            } else {
              return <pre class="content">{content}</pre>;
            }
          },
          [SEMANTIC_PREVIEW_IMAGE_URL]: (
            value: any,
            item: ValueMap
          ): JSX.Element => {
            const img = (
              <img
                src={value}
                style={{ "max-width": "100%", "max-height": "300px" }}
              />
            );
            const url = item[SEMANTIC_URL];
            if (url) {
              return <a href={url}>{img}</a>;
            } else {
              return img;
            }
          },
        },
        attributeFieldRenderers: {
          [SEMANTIC_NOTE_BODY]: renderAttrFieldTextArea,
        },
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
          [TY_SEMANTIC_TAG]: (tag) =>
            tag[SEMANTIC_TAG_NAME] ||
            tag[SEMANTIC_TITLE] ||
            tag[FACTOR_IDENT] ||
            tag[FACTOR_ID] ||
            "???",
        },
      };
    },
  };
}
