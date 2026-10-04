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
  if (name.endsWith(".AppImage")) return { group: "Linux", rank: 0, hint: "免安装，赋予可执行权限后直接运行" };
  if (lower.endsWith(".deb")) return { group: "Linux", rank: 1, hint: "Debian / Ubuntu 等" };
  if (lower.endsWith(".rpm")) return { group: "Linux", rank: 2, hint: "Fedora / RHEL / openSUSE 等" };
  if (lower.endsWith(".apk")) {
    const abi = androidAbi(name);
    const rank = abi === "arm64-v8a" ? 0 : abi === "universal" ? 3 : abi === "armeabi-v7a" ? 1 : 2;
    return { group: "Android", rank, hint: androidHint(abi) };
  }
  return null;
}

function macHint(name) {
  if (/aarch64|arm64/i.test(name)) return "Apple 芯片（M 系列）";
  if (/x86_64|x64|intel/i.test(name)) return "Intel 芯片";
  return "macOS 安装映像";
}

function androidAbi(name) {
  for (const abi of ["arm64-v8a", "armeabi-v7a", "x86_64", "x86", "universal"]) {
    if (name.includes(abi)) return abi;
  }
  return "universal";
}

function androidHint(abi) {
  if (abi === "arm64-v8a") return "绝大多数现代 64 位手机与平板";
  if (abi === "armeabi-v7a") return "较旧的 32 位设备";
  if (abi === "x86_64" || abi === "x86") return "模拟器或少数 x86 设备";
  return "通用包，兼容多数设备但体积更大";
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
console.log(`[notes] wrote ${out} (${rows.length} asset(s) listed) for ${tag}`);
