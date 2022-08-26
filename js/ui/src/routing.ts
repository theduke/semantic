export const ROUTE_BROWSE = "/browse";
export const ROUTE_SETTINGS = "/settings";
export const ROUTE_SETTINGS_TAG_MANAGER = "/settings/tags";
export const ROUTE_SETTINGS_BLOB_CLEANUP = "/settings/blobs/cleanup";
export const ROUTE_SETTINGS_ANALYSE_MEDIA = "/settings/media/analyze";

export function routeEntityPage(ident: string): string {
  return "/entity/" + ident;
}
