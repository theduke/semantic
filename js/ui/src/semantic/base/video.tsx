import { JSX } from "solid-js";
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
  SemanticVideo,
  SEMANTIC_DESCRIPTION,
  TY_SEMANTIC_VIDEO,
} from "semantic/dist/schema";
import { Box } from "solid-bulma";

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
              "max-width": "100%",
              "max-height": "200px",
              "object-fit": "contain",
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
              "max-width": "100%",
              "max-height": "100%",
              "object-fit": "contain",
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
        {renderEntityTable(useRegistry(), item)}
      </div>
    );
  }
}

export function renderVideoMedia(
  props: MediaRenderProps
): [MediaHandle | null, JSX.Element] {
  const info = extractVideoInfo(props.item);

  if (!info?.videoUrl) {
    return [
      null,
      <Box>
        <Notification>Video has no blob or remote url.</Notification>
        {renderEntityTable(useRegistry(), props.item)}
      </Box>,
    ];
  }

  let tag: HTMLVideoElement | undefined;

  const handle: MediaHandle = {
    play() {
      tag?.play();
    },
    pause() {
      tag?.pause();
    },
    mute() {
      if (tag) {
        tag.muted = true;
      }
    },
    unmute() {
      if (tag) {
        tag.muted = false;
      }
    },
    isPlaying() {
      if (tag) {
        return tag.paused === false;
      } else {
        return false;
      }
    },
    isFinished() {
      return tag ? tag.ended : false;
    },
    progress() {
      if (tag) {
        const duration = tag.duration;
        return tag.currentTime / duration;
      } else {
        return null;
      }
    },
  };

  const content = (
    <video
      autoplay={props.autoStart}
      ref={tag}
      controls
      aria-label={info.title}
      poster={info.posterUrl}
      onended={() => {
        props.onFinished();
      }}
      onerror={(_e) => {
        if (tag?.error) {
          props.onFailed(tag.error.message);
        }
      }}
      style={{
        "max-width": "100%",
        "max-height": "100%",
        "object-fit": "contain",
      }}
    >
      <source src={info.videoUrl} />
    </video>
  );

  return [handle, content];
}
