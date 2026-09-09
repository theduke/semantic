import browser from "webextension-polyfill";

import {
  fallbackTitle,
  normalizeWebsiteUrl,
  type BookmarkLink,
} from "./bookmark.js";
import { requiredElement, setStatus } from "./dom.js";
import type { BackgroundRequest, BackgroundResponse } from "./messages.js";

const form = requiredElement("bookmark-form", HTMLFormElement);
const urlInput = requiredElement("url", HTMLInputElement);
const titleInput = requiredElement("title", HTMLInputElement);
const descriptionInput = requiredElement("description", HTMLTextAreaElement);
const submitButton = requiredElement("submit", HTMLButtonElement);
const status = requiredElement("status", HTMLParagraphElement);
const result = requiredElement("result", HTMLAnchorElement);
const configure = requiredElement("configure", HTMLAnchorElement);

let existingBookmark: BookmarkLink | null = null;

configure.addEventListener("click", (event) => {
  event.preventDefault();
  void browser.runtime.openOptionsPage();
});

form.addEventListener("submit", (event) => {
  event.preventDefault();
  if (existingBookmark) {
    void browser.tabs.create({ url: existingBookmark.href });
    return;
  }
  void saveBookmark();
});

void initialize();

async function initialize(): Promise<void> {
  setBusy(true);
  try {
    const configResponse = await send({ type: "config.get" });
    if (!configResponse.ok) throw new Error(configResponse.error);
    if (!("config" in configResponse.data) || !configResponse.data.config) {
      throw new Error(
        "Configure your Semantic application URL to get started.",
      );
    }

    const [tab] = await browser.tabs.query({
      active: true,
      currentWindow: true,
    });
    if (!tab?.url)
      throw new Error("The active tab does not expose a website URL.");
    const url = normalizeWebsiteUrl(tab.url);
    urlInput.value = url;
    titleInput.value = tab.title?.trim() || fallbackTitle(url);
    setStatus(status, "Checking whether this page is already saved…", "busy");

    const lookup = await send({ type: "bookmark.lookup", url });
    if (!lookup.ok) throw new Error(lookup.error);
    if (!("bookmark" in lookup.data))
      throw new Error("Invalid lookup response.");
    existingBookmark = lookup.data.bookmark;
    if (existingBookmark) {
      showBookmark(existingBookmark, "Already saved in Semantic.");
      submitButton.textContent = "Open saved bookmark";
    } else {
      setStatus(status, "Ready to save this page.");
    }
    setBusy(false);
  } catch (error) {
    setStatus(
      status,
      error instanceof Error
        ? error.message
        : "Unable to inspect the active tab.",
      "error",
    );
    setBusy(false, true);
  }
}

async function saveBookmark(): Promise<void> {
  setBusy(true);
  setStatus(status, "Saving bookmark…", "busy");
  result.hidden = true;
  try {
    const response = await send({
      type: "bookmark.capture",
      bookmark: {
        url: urlInput.value,
        title: titleInput.value,
        ...(descriptionInput.value.trim()
          ? { description: descriptionInput.value }
          : {}),
      },
    });
    if (!response.ok) throw new Error(response.error);
    if (!("created" in response.data))
      throw new Error("Invalid save response.");
    const { bookmark, created } = response.data;
    existingBookmark = bookmark;
    showBookmark(
      bookmark,
      created ? "Bookmark saved." : "This page was already saved.",
    );
    submitButton.textContent = "Open saved bookmark";
  } catch (error) {
    setStatus(
      status,
      error instanceof Error ? error.message : "Unable to save the bookmark.",
      "error",
    );
  } finally {
    setBusy(false);
  }
}

function showBookmark(bookmark: BookmarkLink, message: string): void {
  result.href = bookmark.href;
  result.textContent = bookmark.title
    ? `Open “${bookmark.title}” in Semantic`
    : "Open bookmark in Semantic";
  result.hidden = false;
  setStatus(status, message, "success");
}

function setBusy(busy: boolean, disabled = false): void {
  submitButton.disabled = busy || disabled;
  titleInput.disabled = busy || disabled;
  descriptionInput.disabled = busy || disabled;
  form.setAttribute("aria-busy", String(busy));
}

async function send(request: BackgroundRequest): Promise<BackgroundResponse> {
  return browser.runtime.sendMessage(request) as Promise<BackgroundResponse>;
}
