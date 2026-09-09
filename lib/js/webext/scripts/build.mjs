import { mkdir, readFile, readdir, rm, writeFile } from "node:fs/promises";
import { dirname, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { build } from "esbuild";
import { zipSync } from "fflate";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const dist = join(root, "dist");
const staticFiles = ["popup.html", "options.html", "styles.css"];
const targets = {
  chrome: { manifest: "chrome.json", jsTarget: "chrome109" },
  firefox: { manifest: "firefox.json", jsTarget: "firefox140" },
};
const buildDirectories = [];
const bundlePaths = [];

await rm(dist, { recursive: true, force: true });
await mkdir(dist, { recursive: true });

for (const [browser, target] of Object.entries(targets)) {
  const output = join(dist, browser);
  await mkdir(output, { recursive: true });
  buildDirectories.push(output);
  await build({
    entryPoints: {
      background: join(root, "src/background.ts"),
      popup: join(root, "src/popup.ts"),
      options: join(root, "src/options.ts"),
    },
    bundle: true,
    entryNames: "[name]",
    format: "iife",
    legalComments: "eof",
    outdir: output,
    platform: "browser",
    sourcemap: false,
    target: target.jsTarget,
  });

  await writeFile(
    join(output, "manifest.json"),
    await readFile(join(root, "manifests", target.manifest)),
  );
  await Promise.all(
    staticFiles.map(async (name) =>
      writeFile(join(output, name), await readFile(join(root, "src", name))),
    ),
  );

  const archive = {};
  for (const path of await filesWithin(output)) {
    archive[relative(output, path)] = [
      new Uint8Array(await readFile(path)),
      { mtime: new Date("1980-01-01T00:00:00.000Z") },
    ];
  }
  const bundlePath = join(dist, `semantic-${browser}.zip`);
  await writeFile(bundlePath, zipSync(archive, { level: 9 }));
  bundlePaths.push(bundlePath);
}

console.log("Unpacked web extensions:");
for (const buildDirectory of buildDirectories) console.log(buildDirectory);
console.log("Web extension bundles:");
for (const bundlePath of bundlePaths) console.log(bundlePath);

async function filesWithin(directory) {
  const entries = await readdir(directory, { withFileTypes: true });
  const files = [];
  for (const entry of entries.sort((left, right) =>
    left.name.localeCompare(right.name),
  )) {
    const path = join(directory, entry.name);
    if (entry.isDirectory()) files.push(...(await filesWithin(path)));
    else files.push(path);
  }
  return files;
}
