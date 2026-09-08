#!/usr/bin/env node
import { readFile, writeFile } from "node:fs/promises";
import { format } from "prettier";
import { packageModel } from "./package.js";
import { renderPackage } from "./render.js";
import type { Package } from "../types.js";
import { parseJson } from "../json.js";
const [input, output] = process.argv.slice(2);
if (!input || !output) {
  console.error("usage: semantic-ts-generate <package.json> <output.ts>");
  process.exitCode = 2;
} else {
  const pkg = parseJson(await readFile(input, "utf8")) as Package;
  await writeFile(
    output,
    await format(renderPackage(packageModel(pkg)), { parser: "typescript" }),
  );
}
