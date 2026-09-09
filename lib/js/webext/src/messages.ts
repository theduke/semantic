import type { BookmarkInput, BookmarkLink } from "./bookmark.js";
import type { ExtensionConfig } from "./config.js";

export type BackgroundRequest =
  | { type: "config.get" }
  | { type: "bookmark.lookup"; url: string }
  | { type: "bookmark.capture"; bookmark: BookmarkInput };

export type BackgroundResponse =
  | { ok: true; data: { config: ExtensionConfig | null } }
  | { ok: true; data: { bookmark: BookmarkLink | null } }
  | {
      ok: true;
      data: { created: boolean; bookmark: BookmarkLink };
    }
  | { ok: false; error: string };

export function parseBackgroundRequest(
  value: unknown,
): BackgroundRequest | null {
  if (!value || typeof value !== "object") return null;
  const request = value as Record<string, unknown>;
  if (request.type === "config.get") return { type: "config.get" };
  if (request.type === "bookmark.lookup" && typeof request.url === "string") {
    return { type: "bookmark.lookup", url: request.url };
  }
  if (request.type !== "bookmark.capture") return null;
  const bookmark = request.bookmark;
  if (!bookmark || typeof bookmark !== "object") return null;
  const fields = bookmark as Record<string, unknown>;
  if (typeof fields.url !== "string" || typeof fields.title !== "string") {
    return null;
  }
  if (
    fields.description !== undefined &&
    typeof fields.description !== "string"
  ) {
    return null;
  }
  return {
    type: "bookmark.capture",
    bookmark: {
      url: fields.url,
      title: fields.title,
      ...(typeof fields.description === "string"
        ? { description: fields.description }
        : {}),
    },
  };
}

export function errorMessage(error: unknown): string {
  return error instanceof Error
    ? error.message
    : "An unexpected error occurred.";
}
