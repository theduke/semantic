import { Box, Divider, Modal, Notification, Stack } from "@mantine/core";
import { ReactNode, useState } from "react";

import { renderEntityTable } from "../component/entity/generic";
import { useApi, useRegistry } from "../component/context";
import { FACTOR_ID, FACTOR_TYPE, Id, SEMANTIC_DESCRIPTION, SemanticImage, TY_SEMANTIC_IMAGE } from "@semantic/api";
import {
  EntityRenderOpts,
  MediaHandle,
  MediaRenderProps,
  ValueMap,
} from "../registry";

interface ImageInfo {
  imageUrl: string;
  previewUrl?: string;
  title?: string;
}

export function blobImageUrl(id: Id): string {
  return "/blob/image/" + id;
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
): ReactNode {
  return <ImageEntity item={item} opts={opts} />;
}

export function ImageEntity(
  { item, opts }: { item: ValueMap, opts: EntityRenderOpts }
): ReactNode {
  const registry = useRegistry();
  const api = useApi();

  // TODO: typechecks!
  const data: SemanticImage = item as any;
  const id = data[FACTOR_ID];

  let blob = data["semantic/blob_uri_web"] ?? data["semantic/blob_uri"];

  const path = blob
    ? "/blob/image/" + id
    : data["semantic/preview_image_url"] ?? data["semantic/download_url"];


  const alt = data["semantic/title"] ?? item[SEMANTIC_DESCRIPTION];
  const href = api.host + path;

  if (href) {
    if (opts.preview) {
      return imagePreviewModal(href, href, alt);
    } else {
      return (
        <Stack>
          <div>
            <img
              src={href}
              alt={alt}
              style={{
                maxWidth: "100%",
                maxHeight: "100%",
                objectFit: "contain",
              }}
            />
          </div>
          <Divider />
          {renderEntityTable(registry, item)}
        </Stack>
      );
    }
  } else {
    return (
      <div>
        <Notification>Image has no blob or remote url.</Notification>
        {renderEntityTable(registry, item)}
      </div>
    );
  }
}

function imagePreviewModal(
  previewHref: string,
  href: string,
  alt: string | undefined
): ReactNode {
  const [isActive, setIsActive] = useState(false);

  const modal = isActive && (
    <Modal size='auto' onClose={() => setIsActive(false)} opened>
      <div>
        <img
          onClick={() => setIsActive(false)}
          src={href}
          className="is-clickable"
          alt={alt}
          style={{
            maxWidth: "100%",
            maxHeight: "100%",
            objectFit: "fill",
          }}
        />
      </div>
    </Modal>
  );


  return (
    <div>
      <img
        onClick={() => setIsActive(true)}
        src={previewHref}
        alt={alt}
        style={{ maxHeight: "200px", cursor: "pointer", objectFit: "contain", maxWidth: '100%' }}
      />

      {modal}
    </div>
  );
}

export function renderImageMedia(
  props: MediaRenderProps
): [MediaHandle | null, JSX.Element] {
  return [null, <ImageMedia {...props} />]
}

export function ImageMedia(props: MediaRenderProps): ReactNode {
  const info = extractImageInfo(props.item);
  const registry = useRegistry();

  if (!info?.imageUrl) {
    return [
      null,
      <Box>
        <Notification title='Image has no blob or remote url.' />
        {renderEntityTable(registry, props.item)}
      </Box>,
    ];
  }

  return (
    <img
      src={info.imageUrl}
      alt={info.title}
      style={{
        display: "block",
        maxWidth: "100%",
        maxHeight: "100%",
        objectFit: "contain",
      }}
    />
  );
}

