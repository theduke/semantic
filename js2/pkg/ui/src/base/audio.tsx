import { FACTOR_ID, FACTOR_TYPE, SEMANTIC_DESCRIPTION, SemanticAudio, TY_SEMANTIC_AUDIO } from "@semantic/api";
import {
  EntityRenderOpts,
  MediaHandle,
  MediaRenderProps,
  ValueMap,
} from "../registry";
import { renderEntityTable } from "../component/entity/generic";
import { useRegistry } from "../component/context";
import { Box, Notification } from "@mantine/core";
import { useRef } from "react";

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
  const registry = useRegistry();
  const info = extractAudioInfo(item);

  if (!info) {
    return (
      <div>
        <Notification>Audio has no blob or remote url.</Notification>
        {renderEntityTable(registry, item)}
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
  const registry = useRegistry();

  if (!info?.audioUrl) {
    return [
      null,
      <Box>
        <Notification>Audio has no blob or remote url.</Notification>
        {renderEntityTable(registry, props.item)}
      </Box>,
    ];
  }

  let tagRef = useRef<HTMLAudioElement>(null);

  const handle: MediaHandle = {
    play() {
      tagRef.current?.play();
    },
    pause() {
      tagRef.current?.pause();
    },
    mute() {
      if (tagRef.current) {
        tagRef.current.muted = true;
      }
    },
    unmute() {
      if (tagRef.current) {
        tagRef.current.muted = false;
      }
    },
    isPlaying() {
      return tagRef.current?.paused === false;
    },
    isFinished() {
      return tagRef.current?.ended ?? false;
    },
    progress() {
      if (tagRef.current) {
        const duration = tagRef.current.duration;
        return tagRef.current.currentTime / duration;
      } else {
        return null;
      }
    },
  };

  const content = (
    <audio
      autoPlay={props.autoStart}
      preload={"none"}
      controls
      ref={tagRef}
      aria-label={info.title}
      onError={() => {
        if (tagRef.current?.error) {
          props.onFailed(tagRef.current.error?.message);
        }
      }}
      onEnded={() => {
        props.onFinished();
      }}
    >
      <source src={info.audioUrl} />
    </audio>
  );

  return [handle, content];
}
