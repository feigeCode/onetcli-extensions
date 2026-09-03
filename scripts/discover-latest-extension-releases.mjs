#!/usr/bin/env node
// Discover the latest GitHub release tag for every extension that has a
// published release, emitting a GitHub Actions matrix JSON on stdout:
//
//   {"include":[{"id":"dm","kind":"database_driver","tag":"dm-v0.1.5","version":"0.1.5"}, ...]}
//
// Usage:
//   GITHUB_REPOSITORY=feigeCode/navop-extensions node scripts/discover-latest-extension-releases.mjs
//
// Releases are fetched with the `gh` CLI, which must be authenticated against
// the repository. For offline testing, set GH_RELEASES_JSON to a JSON file
// with the same shape as `gh release list --json tagName,publishedAt,isDraft,isPrerelease`.

import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";

const repoRoot = path.resolve(import.meta.dirname, "..");
const repository = process.env.GITHUB_REPOSITORY || "feigeCode/navop-extensions";

const roots = [
  "extensions/ipc",
  "extensions/remote-desktop",
  "extensions/mcp-helper",
  "extensions/acp-agent",
  "extensions/composite",
  "extensions/wasm",
  "extensions/language",
  "extensions/language-bundle",
];

process.stdout.write(`${JSON.stringify(discover())}\n`);

function loadExtensions() {
  const extensions = [];
  for (const root of roots) {
    if (!fs.existsSync(root)) continue;
    for (const name of fs.readdirSync(root)) {
      const file = path.join(root, name, "extension.build.json");
      if (!fs.existsSync(file)) continue;
      const data = JSON.parse(fs.readFileSync(file, "utf8"));
      if (!data.id || !data.releaseTagPrefix) {
        throw new Error(`extension build metadata missing id or releaseTagPrefix: ${file}`);
      }
      extensions.push({
        id: data.id,
        kind: data.kind || "",
        prefix: data.releaseTagPrefix,
      });
    }
  }
  return extensions.sort((a, b) => a.id.localeCompare(b.id));
}

function loadReleases() {
  if (process.env.GH_RELEASES_JSON) {
    return JSON.parse(fs.readFileSync(process.env.GH_RELEASES_JSON, "utf8"));
  }
  const output = execFileSync(
    "gh",
    [
      "release",
      "list",
      "--repo",
      repository,
      "--limit",
      "1000",
      "--json",
      "tagName,publishedAt,isDraft,isPrerelease",
    ],
    { encoding: "utf8", maxBuffer: 16 * 1024 * 1024 },
  );
  return JSON.parse(output);
}

function discover() {
  const extensions = loadExtensions();
  const releases = loadReleases()
    .filter((release) => !release.isDraft)
    .sort((a, b) => new Date(b.publishedAt) - new Date(a.publishedAt));

  const include = [];
  for (const extension of extensions) {
    const matches = releases.filter((release) => release.tagName.startsWith(extension.prefix));
    if (matches.length === 0) continue;
    const stable = matches.find((release) => !release.isPrerelease);
    const latest = stable || matches[0];
    include.push({
      id: extension.id,
      kind: extension.kind,
      tag: latest.tagName,
      version: latest.tagName.slice(extension.prefix.length),
    });
  }
  return { include };
}
