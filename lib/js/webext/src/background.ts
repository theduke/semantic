import { HttpTransport, SemanticClient } from "@semantic/sdk";
import browser from "webextension-polyfill";

import {
  captureBookmarkSerialized,
  findBookmark,
  normalizeWebsiteUrl,
  type BookmarkClientFactory,
} from "./bookmark.js";
import { rpcEndpoint } from "./config.js";
import { loadStoredConfig } from "./extension-storage.js";
import {
  errorMessage,
  parseBackgroundRequest,
  type BackgroundRequest,
  type BackgroundResponse,
} from "./messages.js";

const createClient: BookmarkClientFactory = (endpoint) =>
  new SemanticClient(new HttpTransport(endpoint));

async function dispatch(
  request: BackgroundRequest,
): Promise<BackgroundResponse> {
  if (request.type === "config.get") {
    return { ok: true, data: { config: await loadStoredConfig() } };
  }

  const config = await loadStoredConfig();
  if (!config) {
    return {
      ok: false,
      error: "Configure the Semantic application URL before saving bookmarks.",
    };
  }

  if (request.type === "bookmark.lookup") {
    const client = createClient(rpcEndpoint(config));
    const bookmark = await findBookmark(
      client,
      config,
      normalizeWebsiteUrl(request.url),
    );
    return { ok: true, data: { bookmark } };
  }

  const result = await captureBookmarkSerialized(
    createClient,
    config,
    request.bookmark,
  );
  return { ok: true, data: result };
}

browser.runtime.onMessage.addListener((value: unknown) => {
  const request = parseBackgroundRequest(value);
  if (!request) {
    return Promise.resolve({ ok: false, error: "Invalid extension request." });
  }
  return dispatch(request).catch((error): BackgroundResponse => ({
    ok: false,
    error: errorMessage(error),
  }));
});
