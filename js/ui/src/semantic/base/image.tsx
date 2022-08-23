import { createSignal, JSX, Show } from "solid-js";
import { Modal } from "../../component/bulma/modal";
import { Notification } from "../../component/bulma/notification";
import { renderEntityTable } from "../../component/entity";
import { useRegistry } from "../../context";
import {
  EntityRenderOpts,
  MediaHandle,
  MediaRenderProps,
  ValueMap,
} from "../registry";
import {
  FACTOR_ID,
  FACTOR_TYPE,
  SemanticImage,
  SEMANTIC_DESCRIPTION,
  TY_SEMANTIC_IMAGE,
} from "semantic/dist/schema";
import { Box } from "solid-bulma";

interface ImageInfo {
  imageUrl: string;
  previewUrl?: string;
  title?: string;
}

function extractImageInfo(item: ValueMap): ImageInfo | null {
  // TODO: validation!
  if (item[FACTOR_TYPE] !== TY_SEMANTIC_IMAGE) {
    return null;
  }
  const data: SemanticImage = item as any;

  const id = data[FACTOR_ID];

  const blobUri = data["semantic/blob_uri_web"] ?? data["semantic/blob_uri"];
  const imageUrl = blobUri
    ? "/blob/image/" + id
    : data["semantic/download_url"];

  if (!imageUrl) {
    return null;
  }

  const title = data["semantic/title"] ?? item[SEMANTIC_DESCRIPTION];

  return {
    imageUrl,
    title,
  };
}

export function renderImage(
  item: ValueMap,
  opts: EntityRenderOpts
): JSX.Element {
  // TODO: typechecks!
  const data: SemanticImage = item as any;
  const id = data[FACTOR_ID];

  let blob = data["semantic/blob_uri_web"] ?? data["semantic/blob_uri"];

  const href = blob
    ? "/blob/image/" + id
    : data["semantic/preview_image_url"] ?? data["semantic/download_url"];

  const alt = data["semantic/title"] ?? item[SEMANTIC_DESCRIPTION];

  if (href) {
    if (opts.preview) {
      return imagePreviewModal(href, href, alt);
    } else {
      return (
        <div>
          <img
            src={href}
            alt={alt}
            style={{
              "max-width": "100%",
              "max-height": "100%",
              "object-fit": "contain",
            }}
          />
        </div>
      );
    }
  } else {
    return (
      <div>
        <Notification>Image has no blob or remote url.</Notification>
        {renderEntityTable(useRegistry(), item)}
      </div>
    );
  }
}

function imagePreviewModal(
  previewHref: string,
  href: string,
  alt: string | undefined
): JSX.Element {
  const [isActive, setIsActive] = createSignal<boolean>(false);

  return (
    <div>
      <img
        onclick={() => setIsActive(true)}
        src={previewHref}
        alt={alt}
        style={{ "max-height": "200px", cursor: "pointer" }}
      />

      <Show when={isActive()}>
        <Modal onClose={() => setIsActive(false)} overflow="hidden">
          <div
            style={{
              "max-width": "100%",
              "max-height": "100%",
              display: "flex",
            }}
          >
            <img
              onclick={() => setIsActive(false)}
              src={href}
              class="is-clickable"
              alt={alt}
              style={{
                "max-width": "100%",
                "max-height": "100%",
                "object-fit": "contain",
              }}
            />
          </div>
        </Modal>
      </Show>
    </div>
  );
}

export function renderImageMedia(
  props: MediaRenderProps
): [MediaHandle | null, JSX.Element] {
  const info = extractImageInfo(props.item);

  if (!info?.imageUrl) {
    return [
      null,
      <Box>
        <Notification>Image has no blob or remote url.</Notification>
        {renderEntityTable(useRegistry(), props.item)}
      </Box>,
    ];
  }

  const content = (
    <img
      src={info.imageUrl}
      alt={info.title}
      style={{
        display: "block",
        "max-width": "100%",
        "max-height": "100%",
        "object-fit": "contain",
      }}
    />
  );

  return [null, content];
}
