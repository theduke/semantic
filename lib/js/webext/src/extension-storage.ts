import browser from "webextension-polyfill";

import { loadConfig, type ExtensionConfig } from "./config.js";

export function loadStoredConfig(): Promise<ExtensionConfig | null> {
  return loadConfig(browser.storage.local);
}
