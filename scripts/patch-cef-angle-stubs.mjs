#!/usr/bin/env node
// Work around a `tauri-bundler` bug on Linux.
//
// Since CEF 152 the `libEGL.so` / `libGLESv2.so` ANGLE libraries are no longer
// shipped as separate files (ANGLE is statically linked into `libcef.so`), but
// the bundler still copies them unconditionally from the CEF distribution:
//
//   for f in cef_files { fs::copy(cef_path.join(f), &dest)?; ... }
//
// so packaging fails with `No such file or directory (os error 2)` while the
// `.deb`/`.rpm` is being written. The bundler also runs `strip` on every
// copied `*.so`, which is why the stand-ins below must be real ELF objects.
//
// Upstream fix: https://github.com/tauri-apps/tauri/pull/16338 (open).
// Documented workaround: https://github.com/tauri-apps/tauri/issues/16338
//
// This script is wired to `build.beforeBundleCommand`, which Tauri runs after
// the Rust build (i.e. after the CEF distribution has been downloaded) and
// before the bundles are produced. It is a no-op outside Linux and on builds
// that do not embed CEF, and it never overwrites files that already exist.
//
// Remove this script (and the `beforeBundleCommand` entry in tauri.conf.json)
// once a CLI with PR #16338 merged is available.

import { execFileSync } from "node:child_process";
import {
  chmodSync,
  closeSync,
  copyFileSync,
  existsSync,
  mkdtempSync,
  openSync,
  readdirSync,
  readSync,
  rmSync,
  statSync,
  writeFileSync,
} from "node:fs";
import { homedir, tmpdir } from "node:os";
import { join } from "node:path";

const STUBS = ["libEGL.so", "libGLESv2.so"];

/** Directories the bundler reads from are looked up under this root. */
const root = process.env.CEF_PATH || join(homedir(), ".cache", "tauri-cef");

/** Every directory up to `depth` levels that contains the CEF marker file. */
function findCefDistributions(directory, depth = 3) {
  if (depth < 0 || !existsSync(directory)) return [];
  if (existsSync(join(directory, "libcef.so"))) return [directory];

  const found = [];
  for (const entry of readdirSync(directory, { withFileTypes: true })) {
    if (entry.isDirectory() && !entry.isSymbolicLink()) {
      found.push(...findCefDistributions(join(directory, entry.name), depth - 1));
    }
  }
  return found;
}

/** A minimal, strippable ELF shared object (what `strip` needs to succeed). */
function compileStub(destination) {
  const work = mkdtempSync(join(tmpdir(), "cef-angle-"));
  try {
    const source = join(work, "stub.c");
    const output = join(work, "stub.so");
    writeFileSync(source, "void welkin_cef_angle_stub(void) {}\n");

    for (const compiler of ["cc", "gcc", "clang"]) {
      try {
        execFileSync(compiler, ["-shared", "-fPIC", "-o", output, source], {
          stdio: "ignore",
        });
        if (existsSync(output)) {
          copyFileSync(output, destination);
          return true;
        }
      } catch {
        // Try the next compiler.
      }
    }
    return false;
  } finally {
    rmSync(work, { recursive: true, force: true });
  }
}

/** Fallback when no C compiler is available: reuse a small ELF that CEF ships. */
function copyExistingElf(directory, destination) {
  const candidates = ["chrome-sandbox", "libvulkan.so.1"];
  for (const name of candidates) {
    const source = join(directory, name);
    if (!existsSync(source)) continue;
    // Only reuse things that are actually ELF, otherwise `strip` would fail.
    try {
      const header = Buffer.alloc(4);
      const fd = openSync(source, "r");
      try {
        readSync(fd, header, 0, 4, 0);
      } finally {
        closeSync(fd);
      }
      if (header.toString("latin1") === "\x7fELF") {
        copyFileSync(source, destination);
        return true;
      }
    } catch {
      // Keep looking.
    }
  }
  return false;
}

if (process.platform !== "linux") process.exit(0);

const distributions = findCefDistributions(root);
if (distributions.length === 0) {
  // Not a CEF build (or CEF has not been downloaded); nothing to patch.
  console.log(`[patch-cef-angle] no CEF distribution under ${root}, nothing to do`);
  process.exit(0);
}

let patched = 0;
for (const directory of distributions) {
  for (const name of STUBS) {
    const destination = join(directory, name);
    if (existsSync(destination)) continue;

    const created = compileStub(destination) || copyExistingElf(directory, destination);
    if (!created) {
      console.error(
        `[patch-cef-angle] cannot create the ${name} placeholder in ${directory}: ` +
          "no C compiler and no reusable ELF found. See tauri-apps/tauri#16338.",
      );
      process.exit(1);
    }
    chmodSync(destination, 0o755);
    const size = (statSync(destination).size / 1024).toFixed(1);
    console.log(`[patch-cef-angle] wrote ${name} placeholder (${size} KB) into ${directory}`);
    patched += 1;
  }
}

console.log(
  patched === 0
    ? "[patch-cef-angle] CEF distributions already carry the ANGLE placeholders"
    : `[patch-cef-angle] added ${patched} placeholder(s) for tauri-apps/tauri#16338`,
);
