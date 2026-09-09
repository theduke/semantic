import browser from "webextension-polyfill";

import {
  CONFIG_STORAGE_KEY,
  normalizeBaseUrl,
  originPermission,
  type ExtensionConfig,
} from "./config.js";
import { requiredElement, setStatus } from "./dom.js";
import { loadStoredConfig } from "./extension-storage.js";

const form = requiredElement("config-form", HTMLFormElement);
const baseUrlInput = requiredElement("base-url", HTMLInputElement);
const submitButton = requiredElement("save", HTMLButtonElement);
const status = requiredElement("status", HTMLParagraphElement);

form.addEventListener("submit", (event) => {
  event.preventDefault();
  void saveConfig();
});

void initialize();

async function initialize(): Promise<void> {
  try {
    const config = await loadStoredConfig();
    if (config) baseUrlInput.value = config.baseUrl;
  } catch (error) {
    setStatus(
      status,
      messageFor(error, "Unable to load configuration."),
      "error",
    );
  }
}

async function saveConfig(): Promise<void> {
  let baseUrl: string;
  try {
    baseUrl = normalizeBaseUrl(baseUrlInput.value);
  } catch (error) {
    setStatus(status, messageFor(error, "Invalid Semantic URL."), "error");
    return;
  }

  submitButton.disabled = true;
  form.setAttribute("aria-busy", "true");
  setStatus(status, "Requesting access to the Semantic server…", "busy");
  const newOrigin = originPermission(baseUrl);
  try {
    // Permission requests must be the first asynchronous browser operation made
    // from this user gesture. Requesting an already granted origin is harmless.
    const granted = await browser.permissions.request({ origins: [newOrigin] });
    if (!granted) {
      throw new Error(
        "Server access was not granted. Configuration was not changed.",
      );
    }

    const previous = await loadStoredConfig();
    const config: ExtensionConfig = { baseUrl };
    await browser.storage.local.set({ [CONFIG_STORAGE_KEY]: config });
    baseUrlInput.value = baseUrl;
    setStatus(status, "Configuration saved.", "success");

    if (previous && previous.baseUrl !== baseUrl) {
      const oldOrigin = originPermission(previous.baseUrl);
      if (oldOrigin !== newOrigin) {
        // The new configuration is already durable. Failure to clean up the old
        // grant should not make a successful save look like it was rolled back.
        await browser.permissions
          .remove({ origins: [oldOrigin] })
          .catch(() => false);
      }
    }
  } catch (error) {
    setStatus(
      status,
      messageFor(error, "Unable to save configuration."),
      "error",
    );
  } finally {
    submitButton.disabled = false;
    form.setAttribute("aria-busy", "false");
  }
}

function messageFor(error: unknown, fallback: string): string {
  return error instanceof Error ? error.message : fallback;
}
