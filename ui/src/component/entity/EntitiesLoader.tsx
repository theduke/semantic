import { JSX } from "solid-js/jsx-runtime";
import { useApi } from "../../context";
import { IdOrIdent, Item, Page, Select } from "../../semantic/core";
import { BoundarySuspenseLoader } from "../util/load";

export interface EntititesLoaderProps {
  select: Select;
  children: (page: Page<Item>) => JSX.Element;
}

export function EntitiesLoader(props: EntititesLoaderProps): JSX.Element {
  const api = useApi();

  return (
    <BoundarySuspenseLoader<Page<Item>>
      load={() => api.select(props.select)}
      render={props.children}
    />
  );
}
