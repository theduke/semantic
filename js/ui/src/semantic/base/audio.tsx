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
  SemanticAudio,
  SEMANTIC_DESCRIPTION,
  TY_SEMANTIC_AUDIO,
} from "semantic/dist/schema";
import { Box } from "solid-bulma";

interface AudioInfo {
  audioUrl: string;
  posterUrl?: string;
  title?: string;
}

function extractAudioInfo(item: ValueMap): AudioInfo | null {
  // TODO: validation!
  if (item[FACTOR_TYPE] !== TY_SEMANTIC_AUDIO) {
    return null;
  }
  const data: SemanticAudio = item as any;

  const id = data[FACTOR_ID];

  const blobUri = data["semantic/blob_uri_web"] ?? data["semantic/blob_uri"];
  const audioUrl = blobUri ? "/blob/file/" + id : data["semantic/download_url"];

  if (!audioUrl) {
    return null;
  }

  const posterBlobUri = data["semantic/preview_image_blob_uri"];
  const posterUrl = posterBlobUri
    ? "/blob/preview/" + id
    : data["semantic/preview_image_url"] ?? undefined;

  const title = data["semantic/title"] ?? item[SEMANTIC_DESCRIPTION];

  return {
    audioUrl,
    posterUrl,
    title,
  };
}

export function renderAudio(
  item: ValueMap,
  opts: EntityRenderOpts
): JSX.Element {
  const info = extractAudioInfo(item);

  if (!info) {
    return (
      <div>
        <Notification>Audio has no blob or remote url.</Notification>
        {renderEntityTable(useRegistry(), item)}
      </div>
    );
  }

  if (opts.preview) {
    return (
      <div>
        <audio preload={"none"} controls aria-label={info.title}>
          <source src={info.audioUrl} />
        </audio>
      </div>
    );
  } else {
    return (
      <div>
        <audio preload={"auto"} controls aria-label={info.title}>
          <source src={info.audioUrl} />
        </audio>
      </div>
    );
  }
}

export function renderAudioMedia(
  props: MediaRenderProps
): [MediaHandle | null, JSX.Element] {
  const info = extractAudioInfo(props.item);

  if (!info?.audioUrl) {
    return [
      null,
      <Box>
        <Notification>Audio has no blob or remote url.</Notification>
        {renderEntityTable(useRegistry(), props.item)}
      </Box>,
    ];
  }

  let tag: HTMLAudioElement | undefined;

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
      return tag?.ended ?? false;
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
    <audio
      autoplay={props.autoStart}
      preload={"none"}
      controls
      ref={tag}
      aria-label={info.title}
      onerror={() => {
        if (tag?.error) {
          props.onFailed(tag.error?.message);
        }
      }}
      onended={() => {
        props.onFinished();
      }}
    >
      <source src={info.audioUrl} />
    </audio>
  );

  return [handle, content];
}
