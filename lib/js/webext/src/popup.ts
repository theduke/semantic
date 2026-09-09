import browser from "webextension-polyfill";
import {
  fallbackTitle,
  normalizeWebsiteUrl,
  type BookmarkLink,
} from "./bookmark.js";
import { requiredElement, setStatus } from "./dom.js";
import { MetadataPicker } from "./metadata-picker.js";
import type { BackgroundRequest, BackgroundResponse } from "./messages.js";

const form = requiredElement("bookmark-form", HTMLFormElement);
const urlInput = requiredElement("url", HTMLInputElement);
const titleInput = requiredElement("title", HTMLInputElement);
const descriptionInput = requiredElement("description", HTMLTextAreaElement);
const submitButton = requiredElement("submit", HTMLButtonElement);
const status = requiredElement("status", HTMLParagraphElement);
const result = requiredElement("result", HTMLAnchorElement);
const configure = requiredElement("configure", HTMLButtonElement);
const pickers: MetadataPicker[] = [];
for (const kind of ["directory", "parent", "labels"] as const) {
  pickers.push(
    new MetadataPicker(
      requiredElement(`${kind}-picker`, HTMLDivElement),
      kind,
      async (kind, query) => {
        const response = await send({ type: "metadata.search", kind, query });
        if (!response.ok) throw new Error(response.error);
        if (!("options" in response.data))
          throw new Error("Invalid search response.");
        return response.data.options;
      },
      () => pickers.forEach((picker) => picker.close()),
    ),
  );
}
let existingBookmark: BookmarkLink | null = null;
let busy = false;
configure.addEventListener(
  "click",
  () => void browser.runtime.openOptionsPage(),
);
form.addEventListener("submit", (event) => {
  event.preventDefault();
  if (!busy && !existingBookmark) void saveBookmark();
});
void initialize();

async function initialize(): Promise<void> {
  setBusy(true);
  try {
    const configResponse = await send({ type: "config.get" });
    if (!configResponse.ok) throw new Error(configResponse.error);
    if (!("config" in configResponse.data) || !configResponse.data.config)
      throw new Error(
        "Open settings using the gear above to connect to Semantic.",
      );
    const [tab] = await browser.tabs.query({
      active: true,
      currentWindow: true,
    });
    if (!tab?.url)
      throw new Error("The active tab does not expose a website URL.");
    const url = normalizeWebsiteUrl(tab.url);
    urlInput.value = url;
    titleInput.value = tab.title?.trim() || fallbackTitle(url);
    const parsed = new URL(url);
    requiredElement("page-domain", HTMLElement).textContent = parsed.hostname;
    requiredElement("page-path", HTMLElement).textContent =
      parsed.pathname === "/" && !parsed.search
        ? "Current page"
        : parsed.pathname + parsed.search;
    requiredElement("page-preview", HTMLDivElement).hidden = false;
    requiredElement("page-preview", HTMLDivElement).title = url;
    setStatus(status, "Checking your bookmarks…", "busy");
    const lookup = await send({ type: "bookmark.lookup", url });
    if (!lookup.ok) throw new Error(lookup.error);
    if (!("bookmark" in lookup.data))
      throw new Error("Invalid lookup response.");
    existingBookmark = lookup.data.bookmark;
    if (existingBookmark)
      showBookmark(existingBookmark, "Already in your collection.");
    else {
      form.hidden = false;
      setStatus(status, "");
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
  const directoryId = pickers.find((picker) => picker.kind === "directory")
    ?.selected[0]?.id;
  const parentId = pickers.find((picker) => picker.kind === "parent")
    ?.selected[0]?.id;
  const labelIds = pickers
    .find((picker) => picker.kind === "labels")!
    .selected.map((option) => option.id);
  try {
    const response = await send({
      type: "bookmark.capture",
      bookmark: {
        url: urlInput.value,
        title: titleInput.value,
        ...(descriptionInput.value.trim()
          ? { description: descriptionInput.value }
          : {}),
        ...(directoryId ? { directoryId } : {}),
        ...(parentId ? { parentId } : {}),
        ...(labelIds.length ? { labelIds } : {}),
      },
    });
    if (!response.ok) throw new Error(response.error);
    if (!("created" in response.data))
      throw new Error("Invalid save response.");
    const { bookmark, created, warning } = response.data;
    existingBookmark = bookmark;
    showBookmark(
      bookmark,
      created ? "Saved to your collection." : "Already in your collection.",
    );
    if (warning) setStatus(status, warning, "error");
    result.focus();
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
  form.hidden = true;
  requiredElement("heading", HTMLHeadingElement).textContent =
    "In your collection";
  result.href = bookmark.href;
  requiredElement("result-title", HTMLElement).textContent =
    bookmark.title || titleInput.value || "Saved bookmark";
  result.hidden = false;
  setStatus(status, message, "success");
}

function setBusy(value: boolean, disabled = false): void {
  busy = value;
  submitButton.disabled = value || disabled;
  titleInput.disabled = value || disabled;
  descriptionInput.disabled = value || disabled;
  pickers.forEach((picker) => picker.setDisabled(value || disabled));
  form.setAttribute("aria-busy", String(value));
}

async function send(request: BackgroundRequest): Promise<BackgroundResponse> {
  return browser.runtime.sendMessage(request) as Promise<BackgroundResponse>;
}
