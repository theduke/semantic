#!/usr/bin/env node
import { execFileSync } from "node:child_process";
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { format } from "prettier";
import type { Package } from "../types.js";
import { parseJson } from "../json.js";
import { packageModel } from "./package.js";
import { renderPackage } from "./render.js";

const sdkRoot = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const workspaceRoot = resolve(sdkRoot, "../../..");
const generatedRoot = join(sdkRoot, "src/generated");
const temporary = await mkdtemp(join(tmpdir(), "semantic-sdk-"));

try {
  const outputs = new Map<string, string>();
  const coreOutput = join(temporary, "core.ts");
  execFileSync(
    "cargo",
    ["run", "--quiet", "-p", "semantic_sdk_export", "--", "core", coreOutput],
    { cwd: workspaceRoot, stdio: "inherit" },
  );
  outputs.set(
    "core.ts",
    await format(await readFile(coreOutput, "utf8"), { parser: "typescript" }),
  );
  for (const target of ["base", "filestore"] as const) {
    const input = join(temporary, `${target}.json`);
    execFileSync(
      "cargo",
      ["run", "--quiet", "-p", "semantic_sdk_export", "--", target, input],
      { cwd: workspaceRoot, stdio: "inherit" },
    );
    const pkg = parseJson(await readFile(input, "utf8")) as Package;
    outputs.set(
      `${target}.ts`,
      await format(renderPackage(packageModel(pkg)), { parser: "typescript" }),
    );
  }
  if (process.env.SEMANTIC_GENERATE_CHECK === "1") {
    for (const [name, generated] of outputs) {
      const current = await readFile(join(generatedRoot, name), "utf8");
      if (current !== generated)
        throw new Error(`${name} is stale; run npm run generate`);
    }
  } else {
    await Promise.all(
      [...outputs].map(([name, generated]) =>
        writeFile(join(generatedRoot, name), generated),
      ),
    );
  }
} finally {
  await rm(temporary, { recursive: true, force: true });
}
