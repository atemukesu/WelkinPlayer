#!/usr/bin/env node
/**
 * Copyright 2026 Atemukesu
 * SPDX-License-Identifier: GPL-3.0-only
 */

// Build the GitHub Release body from the staged artifacts: the changelog,
// followed by a per-platform "which file should I download?" guide generated
// from the actual uploaded filenames.
//
// The updater manifest (`latest.json`) deliberately keeps using the raw
// changelog, so the in-app "What's new" dialog stays free of install
// instructions. Only `gh release create/edit --notes-file` uses this output.
//
// Usage: node scripts/compose-release-notes.mjs --dist dist --tag v1.0.1 \
//          [--changelog changelog.md] [--out release-notes.md]

import { existsSync, readFileSync, readdirSync, writeFileSync } from "node:fs";
import { basename, join } from "node:path";

function arg(name, fallback) {
  const index = process.argv.indexOf(`--${name}`);
  return index !== -1 ? process.argv[index + 1] : fallback;
}

const dist = arg("dist", "dist");
const tag = arg("tag");
const changelogFile = arg("changelog", "changelog.md");
const out = arg("out", "release-notes.md");

if (!tag) {
  console.error("usage: node scripts/compose-release-notes.mjs --dist dist --tag v1.0.1 [--changelog changelog.md] [--out release-notes.md]");
  process.exit(1);
}

/**
 * Parse a release tag into a version, tolerating a leading `v` and a SemVer
 * pre-release/build suffix (e.g. `v1.0.3-rc.2`). Returns the bare version or
 * `null` when the tag is not a version at all.
 */
function parseVersion(tag) {
  const match = /^v?(\d+\.\d+\.\d+(?:[-+][0-9A-Za-z.-]+)?)$/.exec(tag.trim());
  return match ? match[1] : null;
}

const version = parseVersion(tag);
if (!version) {
  console.warn(`[notes] warning: tag "${tag}" is not a recognizable version; download hints still apply.`);
}

/** Every uploaded file, by base name (signatures and the manifest excluded). */
function collect(dir) {
  if (!existsSync(dir)) return [];
  const names = [];
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const full = join(dir, entry.name);
    if (entry.isDirectory()) names.push(...collect(full));
    else if (entry.isFile()) names.push(basename(full));
  }
  return names;
}

/** Map one asset name to a platform, sort rank and human description. */
function classify(name) {
  const lower = name.toLowerCase();
  if (lower.endsWith(".sig") || lower === "latest.json") return null;
  if (lower.endsWith(".exe")) return { group: "Windows", rank: 0, hint: "安装向导（NSIS），支持自动更新" };
  if (lower.endsWith(".msi")) return { group: "Windows", rank: 1, hint: "MSI 安装包，适合批量部署" };
  if (lower.endsWith(".dmg")) return { group: "macOS", rank: 0, hint: macHint(name) };
  if (lower.endsWith(".app.tar.gz")) return { group: "macOS", rank: 2, hint: "自动更新用的应用包，无需手动下载" };
  if (lower.endsWith(".appimage")) return { group: "Linux", rank: 0, hint: "免安装，赋予可执行权限后直接运行" };
  if (lower.endsWith(".deb")) return { group: "Linux", rank: 0, hint: "Debian / Ubuntu 等" };
  if (lower.endsWith(".rpm")) return { group: "Linux", rank: 1, hint: "Fedora / RHEL / openSUSE 等" };
  if (lower.endsWith(".apk")) {
    const abi = androidAbi(name);
    const info = ANDROID_ABIS[abi];
    return { group: "Android", rank: info.rank, hint: info.hint };
  }
  return null;
}

function macHint(name) {
  if (/aarch64|arm64/i.test(name)) return "Apple 芯片（M 系列）";
  if (/x86_64|x64|intel/i.test(name)) return "Intel 芯片";
  return "macOS 安装映像";
}

/** Canonical Android ABIs with their sort rank and human description. */
const ANDROID_ABIS = {
  "arm64-v8a": { rank: 0, hint: "绝大多数现代 64 位手机与平板" },
  "armeabi-v7a": { rank: 1, hint: "较旧的 32 位设备" },
  "x86_64": { rank: 2, hint: "模拟器或少数 x86 设备" },
  "x86": { rank: 2, hint: "模拟器或少数 x86 设备" },
  "universal": { rank: 3, hint: "通用包，兼容多数设备但体积更大" },
};

/**
 * Map a raw ABI token (possibly a short form such as `arm`/`arm64`, or a
 * long form such as `arm64-v8a`) to one of the canonical ABIs above.
 */
function canonicalAbi(raw) {
  const token = String(raw).toLowerCase();
  if (token === "arm64-v8a" || token === "arm64") return "arm64-v8a";
  if (token === "armeabi-v7a" || token === "armeabi" || token === "arm") return "armeabi-v7a";
  if (token === "x86_64") return "x86_64";
  if (token === "x86") return "x86";
  return "universal";
}

/**
 * Resolve the canonical Android ABI of an APK asset name.
 *
 * The ABI sits between the versioned tag and `.apk` (e.g.
 * `Welkin-v1.0.3-rc.2-arm64.apk` or `Welkin-v1.0.3-rc.2-arm64-v8a.apk`), so we
 * read the token right before `.apk` first and fall back to a substring scan.
 */
function androidAbi(name) {
  const lower = name.toLowerCase();
  const end = lower.match(/-([a-z0-9_+-]+)\.apk$/);
  if (end) return canonicalAbi(end[1]);
  for (const long of ["arm64-v8a", "armeabi-v7a"]) {
    if (lower.includes(long)) return long;
  }
  if (lower.includes("aarch64") || /\barm64\b/.test(lower)) return "arm64-v8a";
  if (lower.includes("armeabi") || /\barmv7\b/.test(lower) || /\barm\b/.test(lower)) return "armeabi-v7a";
  if (lower.includes("x86_64")) return "x86_64";
  if (lower.includes("x86")) return "x86";
  return "universal";
}

const order = ["Windows", "macOS", "Linux", "Android"];
const groups = new Map(order.map((group) => [group, []]));
for (const name of collect(dist)) {
  const entry = classify(name);
  if (entry) groups.get(entry.group).push({ name, ...entry });
}

for (const files of groups.values()) {
  files.sort((a, b) => a.rank - b.rank || a.name.localeCompare(b.name));
}

const rows = [];
for (const group of order) {
  for (const file of groups.get(group)) {
    const badge = file.rank === 0 ? "**推荐** — " : "";
    rows.push(`| ${group} | \`${file.name}\` | ${badge}${file.hint} |`);
  }
}

const changelog =
  changelogFile && existsSync(changelogFile) ? readFileSync(changelogFile, "utf8").trim() : "";

const guide = [
  "## 下载指引",
  "",
  "请根据你的系统选择对应文件。桌面版安装后会在应用内自动检查更新，通常只需下载一次；" +
    "文件名以 `.sig` 结尾的以及 `latest.json` 仅供自动更新使用，**无需下载**。",
  "",
];

if (rows.length > 0) {
  guide.push("| 平台 | 文件 | 说明 |", "| --- | --- | --- |", ...rows, "");
} else {
  guide.push("> 未在本次构建产物中找到可下载的安装包。", "");
}

guide.push("> 提示：不清楚该选哪个时，各平台标有「**推荐**」的文件即为首选。", "");

const sections = [];
if (changelog) sections.push(changelog, "");
sections.push("---", "", ...guide);

writeFileSync(out, `${sections.join("\n").replace(/\n{3,}/g, "\n\n").trimEnd()}\n`);
console.log(`[notes] wrote ${out} (${rows.length} asset(s) listed) for ${version ?? tag}`);
