import { JSX } from "solid-js";
import { GenericPage } from "../util";
import { Uploader } from "./Uploader";

export function UploadPage(): JSX.Element {
  return (
    <GenericPage title="Upload">
      <Uploader />
    </GenericPage>
  );
}
