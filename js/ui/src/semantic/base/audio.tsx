import { JSX } from "solid-js";
import { Notification } from "../../component/bulma/notification";
import { renderEntityTable } from "../../component/entity";
import { useRegistry } from "../../context";
import { EntityRenderOpts, ValueMap } from "../registry";
import {
  FACTOR_ID,
  SemanticAudio,
  SEMANTIC_DESCRIPTION,
} from "semantic/dist/schema";

export function renderAudio(
  item: ValueMap,
  opts: EntityRenderOpts
): JSX.Element {
  // TODO: typechecks!
  const data: SemanticAudio = item as any;

  const id = data[FACTOR_ID];

  const blobUri = data["semantic/blob_uri_web"] ?? data["semantic/blob_uri"];
  const source = blobUri ? "/blob/file/" + id : data["semantic/download_url"];
  const alt = data["semantic/title"] ?? item[SEMANTIC_DESCRIPTION];

  if (source) {
    if (opts.preview) {
      return (
        <div>
          <audio preload={"none"} controls aria-label={alt}>
            <source src={source} />
          </audio>
        </div>
      );
    } else {
      return (
        <div>
          <audio preload={"auto"} controls aria-label={alt}>
            <source src={source} />
          </audio>
        </div>
      );
    }
  } else {
    return (
      <div>
        <Notification>Audio has no blob or remote url.</Notification>
        {renderEntityTable(useRegistry(), item)}
      </div>
    );
  }
}
