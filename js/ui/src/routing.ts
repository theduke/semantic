export const ROUTE_BROWSE = "/browse";
export const ROUTE_SETTINGS = "/settings";
export const ROUTE_SETTINGS_TAG_MANAGER = "/settings/tags";

export function routeEntityPage(ident: string): string {
  return "/entity/" + ident;
}
