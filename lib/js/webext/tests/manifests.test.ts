import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

const readManifest = async (browser: "chrome" | "firefox") =>
  JSON.parse(
    await readFile(
      new URL(`../manifests/${browser}.json`, import.meta.url),
      "utf8",
    ),
  ) as Record<string, any>;

test("browser manifests use MV3 with minimal required permissions", async () => {
  for (const browser of ["chrome", "firefox"] as const) {
    const manifest = await readManifest(browser);
    assert.equal(manifest.manifest_version, 3);
    assert.equal(manifest.name, "Semantic");
    assert.deepEqual(manifest.permissions, ["activeTab", "storage"]);
    assert.deepEqual(manifest.optional_host_permissions, [
      "http://*/*",
      "https://*/*",
    ]);
    assert.equal(manifest.action.default_popup, "popup.html");
    assert.equal(manifest.options_ui.page, "options.html");
    assert.equal(
      manifest.content_security_policy.extension_pages,
      "script-src 'self'; object-src 'self'",
    );
  }
});

test("each browser uses its supported background declaration", async () => {
  const chrome = await readManifest("chrome");
  const firefox = await readManifest("firefox");
  assert.equal(chrome.background.service_worker, "background.js");
  assert.deepEqual(firefox.background.scripts, ["background.js"]);
  assert.equal(
    firefox.browser_specific_settings.gecko.id,
    "semantic@theduke.dev",
  );
  assert.deepEqual(
    firefox.browser_specific_settings.gecko.data_collection_permissions
      .required,
    ["browsingActivity", "websiteContent"],
  );
});
