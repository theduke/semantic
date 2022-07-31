import { JSX } from "solid-js";
import { NotificationWarning } from "../../component/bulma/notification";
import { renderEntityTable } from "../../component/entity";
import { useRegistry } from "../../context";
import { EntityRenderOpts, ValueMap } from "../registry";
import { FACTOR_ID, SemanticVideo, SEMANTIC_DESCRIPTION } from "semantic/dist/schema";

export function renderVideo(
  item: ValueMap,
  opts: EntityRenderOpts
): JSX.Element {
  // TODO: typechecks!
  const data: SemanticVideo = item as any;

  const id = data[FACTOR_ID];

  const blobUri = data["semantic/blob_uri_web"] ?? data["semantic/blob_uri"];
  const source = blobUri ? "/blob/video/" + id : data["semantic/download_url"];

  const posterBlobUri = data["semantic/preview_image_blob_uri"];
  const posterUri = posterBlobUri
    ? "/blob/preview/" + id
    : data["semantic/preview_image_url"];

  const alt = data["semantic/title"] ?? item[SEMANTIC_DESCRIPTION];

  if (source) {
    if (opts.preview) {
      return (
        <div>
          <video
            preload={"none"}
            controls
            aria-label={alt}
            poster={posterUri ?? undefined}
            style={{
              "max-width": "100%",
              "max-height": "200px",
              "object-fit": "contain",
            }}
          >
            <source src={source} />
          </video>
        </div>
      );
    } else {
      return (
        <div>
          <video
            preload={"auto"}
            controls
            aria-label={alt}
            poster={posterUri ?? undefined}
            style={{
              "max-width": "100%",
              "max-height": "100%",
              "object-fit": "contain",
            }}
          >
            <source src={source} />
          </video>
        </div>
      );
    }
  } else {
    return (
      <div>
        <NotificationWarning>
          Video has no blob or remote url.
        </NotificationWarning>
        {renderEntityTable(useRegistry(), item)}
      </div>
    );
  }
}
