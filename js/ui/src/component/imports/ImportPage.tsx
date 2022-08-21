import { useSearchParams } from "solid-app-router";
import { GenericPage } from "../util";
import { Importer } from "./Importer";

export function ImportPage() {
  const [params, setParams] = useSearchParams();

  const rawUrl = params["url"];
  const url = typeof rawUrl === "string" ? rawUrl : "";
  const rawImportMedia = params["importMedia"];

  let importMedia: boolean;
  switch (rawImportMedia) {
    case "1":
      importMedia = true;
      break;
    case "0":
      importMedia = false;
      break;
    default:
      importMedia = true;
  }

  return (
    <GenericPage title="Import">
      <Importer
        initialSettings={{ url, importMedia }}
        onSettingsChanged={(settings) => {
          setParams({
            url: settings.url,
            importMedia: settings.importMedia ? "1" : "0",
          });
        }}
      />
    </GenericPage>
  );
}
