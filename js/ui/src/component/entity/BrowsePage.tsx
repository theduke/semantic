import { useSearchParams } from "solid-app-router";
import { JSX } from "solid-js";
import { GenericPage } from "../util";
import { EntityBrowser } from "./EntityBrowser";
import { EntityFilter, validateEntityFilter } from "./filter";

const STORAGE_KEY = "browse-page";

export function BrowsePage(): JSX.Element {
  // return <EntityBrowser storageKey={STORAGE_KEY} />

  let initialFilter: EntityFilter | undefined;

  const [searchParams, setSearchParams] = useSearchParams();
  const rawFilter = searchParams["filter"];
  if (typeof rawFilter === "string") {
    try {
      const json = JSON.parse(rawFilter);
      initialFilter = validateEntityFilter.parse(json);
    } catch (e) {
      console.debug("Failed to parse filter", { rawFilter });
    }
  }
  return (
    <GenericPage title="Browse">
      <EntityBrowser
        storageKey={STORAGE_KEY}
        initialFilter={initialFilter}
        onFilterChanged={(filter) => {
          setSearchParams({ filter: JSON.stringify(filter) });
        }}
      />
    </GenericPage>
  );
}
