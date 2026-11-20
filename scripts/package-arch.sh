#!/usr/bin/env bash
# Build an Arch Linux package (.pkg.tar.zst) for Welkin from the .deb produced by
# `pnpm tauri build --bundles deb,rpm`.
#
# Tauri has no pacman bundler, so this repackages the self-contained Debian
# payload (executable + bundled CEF runtime + desktop entry + icons) with
# makepkg, using packaging/arch/PKGBUILD. The result is the same bits that were
# already verified by scripts/verify-cef-bundle.mjs, only in pacman's format.
#
# This script expects to run on Arch Linux (makepkg). CI does that by invoking
# it inside the official `archlinux` container:
#
#   docker run --rm \
#     -v "$PWD:/src:ro" -v "$PWD/dist/arch:/out" \
#     archlinux:base-devel \
#     bash /src/scripts/package-arch.sh --deb-dir /src/src-tauri/target/release/bundle/deb --out /out
#
# When started as root (the container default) it installs base-devel if needed,
# creates an unprivileged build user (makepkg refuses to run as root) and copies
# the finished package to --out. Locally it can be run as your normal user from
# a checkout.
#
# Usage:
#   scripts/package-arch.sh --deb <path/to/Welkin_x.y.z_amd64.deb> [options]
#   scripts/package-arch.sh --deb-dir <dir containing exactly one .deb> [options]
#
# Options:
#   --out <dir>        where to copy the .pkg.tar.zst (default: dist/arch)
#   --version <ver>    override the version derived from the .deb filename
#   --pkgrel <n>       Arch pkgrel (default: 1)

set -euo pipefail

SCRIPT_DIR=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
ROOT=$(cd -- "$SCRIPT_DIR/.." && pwd)
PACKAGING_DIR="$ROOT/packaging/arch"
PKGBUILD_TEMPLATE="$PACKAGING_DIR/PKGBUILD"
INSTALL_FILE="$PACKAGING_DIR/welkinplayer-bin.install"
LICENSE_FILE="$ROOT/LICENSE"
BUILD_USER=welkin-builder

die() {
  echo "[arch] error: $*" >&2
  exit 1
}

info() {
  echo "[arch] $*"
}

usage() {
  sed -n '2,/^$/p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//' | sed '/^$/d'
}

deb=""
deb_dir=""
out="$ROOT/dist/arch"
version=""
pkgrel=1

while [[ $# -gt 0 ]]; do
  case "$1" in
    --deb)
      deb=${2:?--deb needs a path}
      shift 2
      ;;
    --deb-dir)
      deb_dir=${2:?--deb-dir needs a path}
      shift 2
      ;;
    --out)
      out=${2:?--out needs a path}
      shift 2
      ;;
    --version)
      version=${2:?--version needs a value}
      shift 2
      ;;
    --pkgrel)
      pkgrel=${2:?--pkgrel needs a value}
      shift 2
      ;;
    -h | --help)
      usage
      exit 0
      ;;
    *)
      die "unknown argument: $1 (see --help)"
      ;;
  esac
done

[[ -f $PKGBUILD_TEMPLATE ]] || die "PKGBUILD template not found: $PKGBUILD_TEMPLATE"
[[ -f $INSTALL_FILE ]] || die "install script not found: $INSTALL_FILE"
[[ -f $LICENSE_FILE ]] || die "license not found: $LICENSE_FILE"

if [[ -n $deb_dir ]]; then
  [[ -d $deb_dir ]] || die "not a directory: $deb_dir"
  mapfile -t candidates < <(find "$deb_dir" -maxdepth 1 -type f -name '*.deb' | sort)
  [[ ${#candidates[@]} -gt 0 ]] || die "no .deb found under $deb_dir"
  [[ ${#candidates[@]} -eq 1 ]] || die "expected exactly one .deb under $deb_dir, found ${#candidates[@]}"
  deb=${candidates[0]}
fi

[[ -n $deb ]] || die "pass --deb <file> or --deb-dir <dir>"
[[ -f $deb ]] || die "not a file: $deb"
deb=$(cd -- "$(dirname -- "$deb")" && pwd)/$(basename -- "$deb")

# Welkin_1.4.0-rc.1_amd64.deb -> 1.4.0-rc.1
if [[ -z $version ]]; then
  version=$(basename -- "$deb" | sed -nE 's/^.*_([0-9][^_]*)_[^_]+\.deb$/\1/p')
fi
[[ -n $version ]] || die "could not derive the version from $(basename "$deb"); pass --version"

# Arch pkgver allows only [A-Za-z0-9._+], and makepkg's vercmp treats
# "1.4.0rc.1" as older than "1.4.0" whereas "1.4.0.rc.1" would sort newer.
# Dropping the SemVer hyphen is the convention pacman expects for pre-releases.
archver=${version#v}
archver=${archver//-/}
archver=${archver//[^A-Za-z0-9.+_]/}
[[ -n $archver ]] || die "version '$version' cannot be mapped to an Arch pkgver"
[[ $pkgrel =~ ^[0-9]+(\.[0-9]+)?$ ]] || die "--pkgrel must be numeric (got '$pkgrel')"

command -v sha256sum >/dev/null 2>&1 || die "sha256sum is required"
deb_sha256=$(sha256sum "$deb" | awk '{print $1}')
license_sha256=$(sha256sum "$LICENSE_FILE" | awk '{print $1}')

work=$(mktemp -d "${TMPDIR:-/tmp}/welkin-arch.XXXXXXXX")
trap 'rm -rf "$work"' EXIT

install -m 0644 "$PKGBUILD_TEMPLATE" "$work/PKGBUILD"
install -m 0644 "$INSTALL_FILE" "$work/welkinplayer-bin.install"
install -m 0644 "$LICENSE_FILE" "$work/LICENSE-upstream"
cp -- "$deb" "$work/welkin.deb"

export WELKIN_PKGVER="$archver"
export WELKIN_PKGREL="$pkgrel"
export WELKIN_TAG="v$version"
export WELKIN_DEB_SHA256="$deb_sha256"
export WELKIN_LICENSE_SHA256="$license_sha256"
export WELKIN_LOCAL_DEB=1

info "deb:     $(basename -- "$deb")"
info "pkgver:  $archver-$pkgrel"
info "sha256:  $deb_sha256"

if [[ $EUID -eq 0 ]]; then
  if ! command -v makepkg >/dev/null 2>&1; then
    info "installing base-devel (makepkg, fakeroot, binutils, libarchive)"
    pacman -Syu --noconfirm --needed --noprogressbar base-devel
  fi
  id -u "$BUILD_USER" >/dev/null 2>&1 ||
    useradd --create-home --shell /bin/bash "$BUILD_USER"
  chown -R "$BUILD_USER:" "$work"
  info "running makepkg as $BUILD_USER"
  # makepkg refuses to run with uid 0, hence the dedicated build user. The
  # WELKIN_* variables are passed explicitly because su may reset the
  # environment depending on its build/configuration.
  makepkg_cmd=$(printf 'cd %q && env WELKIN_PKGVER=%q WELKIN_PKGREL=%q WELKIN_TAG=%q WELKIN_DEB_SHA256=%q WELKIN_LICENSE_SHA256=%q WELKIN_LOCAL_DEB=1 makepkg --nodeps --noconfirm' \
    "$work" "$archver" "$pkgrel" "v$version" "$deb_sha256" "$license_sha256")
  su -s /bin/bash "$BUILD_USER" -c "$makepkg_cmd"
else
  command -v makepkg >/dev/null 2>&1 ||
    die "makepkg not found: run this on Arch Linux (or inside an archlinux container)"
  ( cd "$work" && makepkg --nodeps --noconfirm )
fi

shopt -s nullglob
packages=("$work"/*.pkg.tar.zst)
[[ ${#packages[@]} -gt 0 ]] || die "makepkg produced no .pkg.tar.zst"

mkdir -p "$out"
for package in "${packages[@]}"; do
  # Gate the pacman package the same way as the .deb/.rpm: a package that only
  # holds the launcher (no libcef.so) installs fine and then dies at startup.
  if ! bsdtar -tf "$package" | grep -qE '/libcef\.so$'; then
    die "$(basename "$package") does not contain the CEF runtime (libcef.so)"
  fi
  cp -f -- "$package" "$out"/
  info "wrote $(cd -- "$out" && pwd)/$(basename -- "$package")"
done
