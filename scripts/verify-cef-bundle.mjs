#!/usr/bin/env node
// Fail the Linux build when the packaged bundle does not actually contain the
// CEF (Chromium Embedded Framework) runtime. `tauri build` succeeds either way:
// when the bundler does not detect the CEF runtime it still produces a valid
// ~12 MB .deb/.rpm that only holds the executable and its desktop entry. That
// package installs cleanly and then dies at launch with a missing `libcef.so`,
// so the mistake only surfaces for users. Guard against that regression here.
//
// A .deb is read with `ar` + `tar` (always present) rather than `dpkg-deb`, so
// this runs both on the CI runner and on a developer machine without dpkg.
//
// Usage: node scripts/verify-cef-bundle.mjs --bundle <dir>

import { execFileSync } from "node:child_process";
import { existsSync, mkdtempSync, readdirSync, rmSync, statSync } from "node:fs";
import { tmpdir } from "node:os";
import { basename, join, resolve } from "node:path";

function arg(name) {
  const index = process.argv.indexOf(`--${name}`);
  return index !== -1 ? process.argv[index + 1] : undefined;
}

/** Files the bundler must copy next to the executable for CEF to start. */
const REQUIRED = [
  // The one file whose absence proves the CEF runtime was not bundled at all.
  "libcef.so",
  "icudtl.dat",
  "v8_context_snapshot.bin",
  "resources.pak",
  "locales/en-US.pak",
];

/** Also shipped by the bundler, but not fatal if a release drops one. */
const EXPECTED = [
  "chrome_100_percent.pak",
  "chrome_200_percent.pak",
  "libvk_swiftshader.so",
  "vk_swiftshader_icd.json",
  "chrome-sandbox",
  // `libEGL.so` / `libGLESv2.so` are deliberately absent: CEF >= 152 links ANGLE
  // into `libcef.so` and no longer ships them. scripts/patch-cef-angle-stubs.mjs
  // creates inert placeholders only so the bundler stops failing on the copy.
];

const bundle = arg("bundle");
if (!bundle) {
  console.error("usage: node scripts/verify-cef-bundle.mjs --bundle <dir>");
  process.exit(1);
}
if (!existsSync(bundle)) {
  console.error(`[verify-cef] bundle directory not found: ${bundle}`);
  process.exit(1);
}

/** Package files found under a subdirectory of the bundle, e.g. `deb`/`rpm`. */
function packagesIn(kind, extension) {
  const dir = join(bundle, kind);
  if (!existsSync(dir)) return [];
  return readdirSync(dir)
    .filter((name) => name.endsWith(extension))
    .map((name) => join(dir, name));
}

function run(command, args, options) {
  return execFileSync(command, args, {
    encoding: "utf8",
    maxBuffer: 512 * 1024 * 1024,
    ...options,
  });
}

/** Paths inside a .deb, relative to the filesystem root (no leading `./` or `/`). */
function listDebFiles(file) {
  const work = mkdtempSync(join(tmpdir(), "verify-cef-"));
  try {
    // `ar t`/`ar x` are part of binutils and always available. `cwd` changes
    // below, so hand `ar` an absolute path (the bundle argument may be relative).
    run("ar", ["x", resolve(file)], { cwd: work });
    const dataTar = readdirSync(work).find((name) => name.startsWith("data.tar"));
    if (!dataTar) throw new Error("no data.tar member in archive");

    // GNU tar auto-detects the compression for reading.
    const output = run("tar", ["tf", dataTar], { cwd: work });
    return output
      .split("\n")
      .map((line) => line.trim().replace(/^\.\//, ""))
      .filter(Boolean);
  } finally {
    rmSync(work, { recursive: true, force: true });
  }
}

/** Paths inside an .rpm, or `null` when the rpm tooling is unavailable. */
function listRpmFiles(file) {
  try {
    const output = run("rpm", ["-qpl", file]);
    return output
      .split("\n")
      .map((line) => line.trim().replace(/^\//, ""))
      .filter(Boolean);
  } catch (error) {
    if (error.code === "ENOENT") return null;
    throw error;
  }
}

let failed = false;

for (const [kind, extension] of [
  ["deb", ".deb"],
  ["rpm", ".rpm"],
]) {
  const packages = packagesIn(kind, extension);
  if (packages.length === 0) {
    // The Linux jobs always build `--bundles deb,rpm`; missing a format is
    // reported but not treated as fatal, the other format still gates the build.
    console.log(`[verify-cef] no ${extension} found under ${join(bundle, kind)}, skipping`);
    continue;
  }

  for (const file of packages) {
    let files;
    try {
      files = extension === ".deb" ? listDebFiles(file) : listRpmFiles(file);
    } catch (error) {
      console.error(`[verify-cef] cannot read ${file}: ${error.message}`);
      failed = true;
      continue;
    }

    if (files === null) {
      console.log(`[verify-cef] rpm tooling not installed, skipping ${basename(file)}`);
      continue;
    }

    // Entries are paths relative to the filesystem root, e.g.
    // `usr/share/Welkin/locales/en-US.pak`, so match on the trailing path
    // segments to cover both `libcef.so` and `locales/en-US.pak`.
    const has = (name) =>
      files.some((path) => path === name || path.endsWith(`/${name}`));

    const missing = REQUIRED.filter((name) => !has(name));
    const absent = EXPECTED.filter((name) => !has(name));
    const size = (statSync(file).size / 1024 / 1024).toFixed(1);
    const found = REQUIRED.filter((name) => has(name));

    console.log(`[verify-cef] ${basename(file)}: ${files.length} entries, ${size} MB`);
    console.log(`[verify-cef]   required CEF files present: ${found.join(", ")}`);
    for (const name of absent) console.log(`[verify-cef]   warning: optional CEF file not bundled: ${name}`);

    if (missing.length > 0) {
      console.error(
        `[verify-cef] FAILED: ${basename(file)} is missing required CEF files: ${missing.join(", ")}`,
      );
      console.error(
        "[verify-cef] The bundle was built without the CEF runtime. Check that the Linux job " +
          "uses @tauri-apps/cli-cef >= 3.0.0-alpha.27 and that the CLI detects " +
          "`tauri-runtime-cef` in src-tauri/Cargo.toml (the bundler only embeds CEF when it does).",
      );
      failed = true;
    }
  }
}

if (failed) process.exit(1);
console.log("[verify-cef] all bundles contain the CEF runtime");
