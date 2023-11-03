import { FACTOR_ID, FACTOR_TYPE, SEMANTIC_DESCRIPTION, SemanticVideo, TY_SEMANTIC_VIDEO } from "@semantic/api";
import {
  EntityRenderOpts,
  MediaHandle,
  MediaRenderProps,
  ValueMap,
} from "../registry";
import { Box, Notification } from "@mantine/core";
import { renderEntityTable } from "../component/entity/generic";
import { useRegistry } from "../component/context";
import { useRef } from "react";

interface VideoInfo {
  videoUrl: string;
  posterUrl?: string;
  title?: string;
}

function extractVideoInfo(item: ValueMap): VideoInfo | null {
  // TODO: validation!
  if (item[FACTOR_TYPE] !== TY_SEMANTIC_VIDEO) {
    return null;
  }
  const data: SemanticVideo = item as any;

  const id = data[FACTOR_ID];

  const blobUri = data["semantic/blob_uri_web"] ?? data["semantic/blob_uri"];
  const videoUrl = blobUri
    ? "/blob/video/" + id
    : data["semantic/download_url"];

  if (!videoUrl) {
    return null;
  }

  const posterBlobUri = data["semantic/preview_image_blob_uri"];
  const posterUrl = posterBlobUri
    ? "/blob/preview/" + id
    : data["semantic/preview_image_url"] ?? undefined;

  const title = data["semantic/title"] ?? item[SEMANTIC_DESCRIPTION];

  return {
    videoUrl,
    posterUrl,
    title,
  };
}

export function renderVideo(
  item: ValueMap,
  opts: EntityRenderOpts
): JSX.Element {
  const info = extractVideoInfo(item);
  const registry = useRegistry();

  if (info?.videoUrl) {
    if (opts.preview) {
      return (
        <div>
          <video
            preload={"none"}
            controls
            aria-label={info.title}
            poster={info.posterUrl}
            style={{
              maxWidth: "100%",
              maxHeight: "200px",
              objectFit: "contain",
            }}
          >
            <source src={info.videoUrl} />
          </video>
        </div>
      );
    } else {
      return (
        <div>
          <video
            preload={"auto"}
            controls
            aria-label={info.title}
            poster={info.posterUrl}
            style={{
              maxWidth: "100%",
              maxHeight: "100%",
              objectFit: "contain",
            }}
          >
            <source src={info.videoUrl} />
          </video>
        </div>
      );
    }
  } else {
    return (
      <div>
        <Notification>Video has no blob or remote url.</Notification>
        {renderEntityTable(registry, item)}
      </div>
    );
  }
}

export function renderVideoMedia(
  props: MediaRenderProps
): [MediaHandle | null, JSX.Element] {
  const info = extractVideoInfo(props.item);
  const registry = useRegistry();

  if (!info?.videoUrl) {
    return [
      null,
      <Box>
        <Notification title='Video has no blob or remote url.' />
        {renderEntityTable(registry, props.item)}
      </Box>,
    ];
  }

  let tatagRef = useRef<HTMLVideoElement>(null);

  const handle: MediaHandle = {
    play() {
      tatagRef.current?.play();
    },
    pause() {
      tatagRef.current?.pause();
    },
    mute() {
      if (tatagRef.current) {
        tatagRef.current.muted = true;
      }
    },
    unmute() {
      if (tatagRef.current) {
        tatagRef.current.muted = false;
      }
    },
    isPlaying() {
      if (tatagRef.current) {
        return tatagRef.current.paused === false;
      } else {
        return false;
      }
    },
    isFinished() {
      return tatagRef.current?.ended ?? false;
    },
    progress() {
      if (tatagRef.current) {
        const duration = tatagRef.current.duration;
        return tatagRef.current.currentTime / duration;
      } else {
        return null;
      }
    },
  };

  const content = (
    <video
      autoPlay={props.autoStart}
      ref={tatagRef}
      controls
      aria-label={info.title}
      poster={info.posterUrl}
      onEnded={() => {
        props.onFinished();
      }}
      onError={() => {
        if (tatagRef.current?.error) {
          props.onFailed(tatagRef.current.error.message);
        }
      }}
      style={{
        maxWidth: "100%",
        maxHeight: "100%",
        objectFit: "contain",
      }}
    >
      <source src={info.videoUrl} />
    </video>
  );

  return [handle, content];
}
