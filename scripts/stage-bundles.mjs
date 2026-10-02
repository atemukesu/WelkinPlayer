#!/usr/bin/env node
// Stage the installer/updater files Tauri produced for one platform into a flat
// folder that will be uploaded as a build artifact. Keeping the staging here
// (instead of relying on upload-artifact globs) gives the release job a stable,
// predictable layout to turn into latest.json.
//
// Usage: node scripts/stage-bundles.mjs --bundle <dir> --out <dir> --os <windows|macos|linux> --key <os-arch>

import { cpSync, existsSync, mkdirSync, readdirSync, renameSync } from "node:fs";
import { join } from "node:path";

function arg(name) {
  const index = process.argv.indexOf(`--${name}`);
  return index !== -1 ? process.argv[index + 1] : undefined;
}

const bundle = arg("bundle");
const out = arg("out");
const os = arg("os");
const key = arg("key") ?? os;
if (!bundle || !out || !os) {
  console.error("usage: node scripts/stage-bundles.mjs --bundle <dir> --out <dir> --os <windows|macos|linux> --key <os-arch>");
  process.exit(1);
}

/** Extensions worth publishing, per OS. */
const WANTED = {
  windows: [".exe", ".exe.sig", ".msi", ".msi.sig"],
  macos: [".app.tar.gz", ".app.tar.gz.sig", ".dmg"],
  linux: [".AppImage", ".AppImage.sig", ".deb", ".rpm"],
};

mkdirSync(out, { recursive: true });

let copied = 0;
function walk(dir) {
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const full = join(dir, entry.name);
    if (entry.isDirectory()) {
      walk(full);
      continue;
    }
    if (WANTED[os].some((extension) => entry.name.endsWith(extension))) {
      cpSync(full, join(out, entry.name));
      copied += 1;
    }
  }
}

if (!existsSync(bundle)) {
  console.error(`[stage] bundle directory not found: ${bundle}`);
  process.exit(1);
}
walk(bundle);

// The macOS updater archive is always named `Welkin.app.tar.gz` regardless of
// architecture, so two matrix legs would collide on the release. Tag it with
// the arch.
if (os === "macos") {
  const arch = key.startsWith("darwin-") ? key.slice("darwin-".length) : key;
  for (const name of readdirSync(out)) {
    if (name.endsWith(".app.tar.gz") || name.endsWith(".app.tar.gz.sig")) {
      renameSync(join(out, name), join(out, name.replace(".app.tar.gz", `_${arch}.app.tar.gz`)));
    }
  }
}

console.log(`[stage] ${key}: copied ${copied} file(s) → ${out}`);
if (copied === 0) {
  console.error(`[stage] no publishable files found under ${bundle}`);
  process.exit(1);
}
