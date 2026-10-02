#!/usr/bin/env node
// Build the Tauri updater manifest (`latest.json`) from the staged artifacts.
//
// The manifest is normally produced by `tauri-action`; when the release is done
// by a separate collect job this script replaces it. The layout it expects is
// the one `scripts/stage-bundles.mjs` writes:
//
//   dist/
//     desktop-windows-x86_64/  Welkin-setup.exe{,.sig} ...
//     desktop-darwin-aarch64/  Welkin_aarch64.app.tar.gz{,.sig} ...
//     desktop-linux-x86_64/    Welkin.AppImage{,.sig} ...
//
// Usage: node scripts/make-latest-json.mjs --dist dist --repo owner/repo \
//          --tag v1.0.0 --version 1.0.0 --notes-file changelog.md --out latest.json

import { existsSync, readFileSync, readdirSync, statSync, writeFileSync } from "node:fs";
import { join } from "node:path";

function arg(name, fallback) {
  const index = process.argv.indexOf(`--${name}`);
  return index !== -1 ? process.argv[index + 1] : fallback;
}

const dist = arg("dist", "dist");
const repo = arg("repo");
const tag = arg("tag");
const version = arg("version");
const notesFile = arg("notes-file");
const out = arg("out", "latest.json");

if (!repo || !tag || !version) {
  console.error("usage: node scripts/make-latest-json.mjs --dist dist --repo owner/repo --tag v1.0.0 --version 1.0.0 [--notes-file file] [--out latest.json]");
  process.exit(1);
}

/** Updater artifact extension to prefer for each OS. */
const PREFERRED = {
  windows: [".exe", ".msi"],
  darwin: [".app.tar.gz"],
  linux: [".AppImage"],
};

const platforms = {};
for (const entry of readdirSync(dist, { withFileTypes: true })) {
  if (!entry.isDirectory()) continue;
  const match = /^desktop-(.+)$/.exec(entry.name);
  if (!match) continue;
  const key = match[1];
  const os = key.split("-")[0];
  const dir = join(dist, entry.name);
  const files = readdirSync(dir);

  const asset = (PREFERRED[os] ?? [])
    .map((extension) => files.find((name) => name.endsWith(extension) && !name.endsWith(".sig")))
    .find(Boolean);
  if (!asset) {
    console.warn(`[latest] no updater asset for ${key}, skipping`);
    continue;
  }
  const signatureName = `${asset}.sig`;
  if (!files.includes(signatureName)) {
    console.warn(`[latest] missing signature ${signatureName} for ${key}, skipping`);
    continue;
  }

  platforms[key] = {
    signature: readFileSync(join(dir, signatureName), "utf8").trim(),
    url: `https://github.com/${repo}/releases/download/${encodeURIComponent(tag)}/${encodeURIComponent(asset)}`,
  };
}

const keys = Object.keys(platforms);
if (keys.length === 0) {
  console.error("[latest] no platforms found; refusing to write an empty manifest");
  process.exit(1);
}

const notes = notesFile && existsSync(notesFile) ? readFileSync(notesFile, "utf8").trim() : "";
writeFileSync(out, `${JSON.stringify({ version, notes, pub_date: new Date().toISOString(), platforms }, null, 2)}\n`);
console.log(`[latest] wrote ${out} (v${version}) with platforms: ${keys.join(", ")}`);
