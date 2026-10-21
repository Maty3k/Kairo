#!/bin/bash
# SPDX-License-Identifier: GPL-3.0-or-later
# tools/mac/setup.sh: install the Mac toolchain for Kairo (docs/PLAN.md section 3.1 and M0).
#
#   tools/mac/setup.sh                 check the disk, show the plan, ask, then install
#   tools/mac/setup.sh --dry-run       check the disk and show the plan only
#   tools/mac/setup.sh --yes [--accept-microsoft-license]
#                                      no questions; pass these only after the owner said yes
#
# It needs at least 20 GB free and downloads a few GB. Steps that are already done are skipped:
#   1. rustup 1.29.1 (rustup-init checked against a pinned SHA-256) with no default toolchain,
#      then the toolchain from rust-toolchain.toml plus the x86_64-pc-windows-msvc target.
#   2. Homebrew llvm (clang-cl, llvm-lib, llvm-rc, libclang). cargo-xwin finds the keg itself.
#   3. Homebrew fnm and Node.js 24.22.1 for this repo only: nothing changes in your shell, and
#      your global Node stays the default. pnpm 12.14.0 comes from that Node's corepack.
#   4. Only after you accept Microsoft's licence for the CRT and Windows SDK files: cargo-xwin
#      0.23.1 and those files (x86_64 only).
# Never uses sudo. Contact: https://github.com/Maty3k/Kairo
# Written for macOS's /bin/bash 3.2.

set -u
set -o pipefail

RUSTUP_VERSION=1.29.1
RUSTUP_SHA256_AARCH64=ec1b9233e7f72990ecd8e62063fa7f6c3dfc2bec8e97f88bff165f9100ac696a  # 2026-10-21: archive/1.29.1/aarch64-apple-darwin
RUSTUP_SHA256_X86_64=259e2b84274434085163fe8d556510571772cda2aa6d87ca6aa664f57bc644e3   # 2026-10-21: archive/1.29.1/x86_64-apple-darwin
DEFAULT_RUST_TOOLCHAIN=1.99.1            # docs/PLAN.md section 2.11, if rust-toolchain.toml is missing
WINDOWS_TARGET=x86_64-pc-windows-msvc
CARGO_XWIN_VERSION=0.23.1
DEFAULT_NODE_VERSION=24.22.1             # docs/PLAN.md section 2.11, if .node-version is missing
PNPM_VERSION=12.14.0
MIN_FREE_GIB=20
MS_LICENSE_URL='https://go.microsoft.com/fwlink/?LinkId=2086102'
USER_AGENT='Kairo-setup (+https://github.com/Maty3k/Kairo)'

SELF_DIR=$(cd "$(dirname "$0")" && pwd -P) || exit 1
REPO_ROOT=$(cd "$SELF_DIR/../.." && pwd -P) || exit 1
TMP_BASE=${TMPDIR:-/tmp}
TMP_BASE=${TMP_BASE%/}
CARGO_BIN="$HOME/.cargo/bin"

say() { printf '%s\n' "$*"; }
step() { printf '\n==> %s\n' "$*"; }
die() {
    printf 'setup: %s\n' "$1" >&2
    exit "${2:-1}"
}

DRY_RUN=0
ASSUME_YES=0
ACCEPT_LICENSE=0
while [ $# -gt 0 ]; do
    case "$1" in
    --dry-run) DRY_RUN=1 ;;
    --yes) ASSUME_YES=1 ;;
    --accept-microsoft-license) ACCEPT_LICENSE=1 ;;
    -h | --help)
        sed -n '5,9p' "$0" | sed 's/^# \{0,1\}//'
        exit 0
        ;;
    *) die "unknown option $1 (see --help)" 2 ;;
    esac
    shift
done

WORK=''
# shellcheck disable=SC2317,SC2329 # called by the EXIT trap below
cleanup() { [ -n "$WORK" ] && rm -rf "$WORK"; }
trap cleanup EXIT

# ------------------------------------------------------------------------------------------
# Checks before anything else
# ------------------------------------------------------------------------------------------
[ "$(uname -s)" = Darwin ] || die "this script is for macOS"
[ "$(id -u)" != 0 ] || die "do not run this with sudo; it installs into your home folder and Homebrew"

free_kib=$(df -Pk "$HOME" | awk 'NR == 2 { print $4 }')
case "$free_kib" in '' | *[!0-9]*) die "could not read the free disk space of $HOME" ;; esac
free_gib=$((free_kib / 1048576))
need_kib=$((MIN_FREE_GIB * 1048576))
say "free disk space for $HOME: $free_gib GiB (needed: $MIN_FREE_GIB GiB)"
if [ "$free_kib" -lt "$need_kib" ]; then
    message="not enough free disk space: $free_gib GiB free, $MIN_FREE_GIB GiB needed. Free some space (for example empty the Trash, old Xcode simulators or large downloads) and run this again."
    [ "$DRY_RUN" = 1 ] || die "$message"
    say "WARNING: $message"
fi

case "$(uname -m)" in
arm64)
    HOST_TRIPLE=aarch64-apple-darwin
    RUSTUP_SHA256=$RUSTUP_SHA256_AARCH64
    ;;
x86_64)
    HOST_TRIPLE=x86_64-apple-darwin
    RUSTUP_SHA256=$RUSTUP_SHA256_X86_64
    ;;
*) die "unsupported Mac architecture: $(uname -m)" ;;
esac

BREW=$(command -v brew 2>/dev/null)
if [ -z "$BREW" ]; then
    for candidate in /opt/homebrew/bin/brew /usr/local/bin/brew; do
        [ -x "$candidate" ] && BREW=$candidate && break
    done
fi
[ -n "$BREW" ] || die "Homebrew is not installed. Install it from https://brew.sh (it asks for your password), then run this again."

RUST_TOOLCHAIN=$DEFAULT_RUST_TOOLCHAIN
toolchain_source="docs/PLAN.md (no rust-toolchain.toml found)"
if [ -f "$REPO_ROOT/rust-toolchain.toml" ]; then
    channel=$(sed -n 's/^[[:space:]]*channel[[:space:]]*=[[:space:]]*"\([^"]*\)".*/\1/p' "$REPO_ROOT/rust-toolchain.toml" | head -n 1)
    if [ -n "$channel" ]; then
        RUST_TOOLCHAIN=$channel
        toolchain_source="rust-toolchain.toml"
    fi
fi
case "$RUST_TOOLCHAIN" in '' | *[!0-9A-Za-z.-]*) die "unexpected toolchain name in rust-toolchain.toml: $RUST_TOOLCHAIN" ;; esac

NODE_VERSION=$DEFAULT_NODE_VERSION
node_source="docs/PLAN.md (no .node-version found)"
if [ -f "$REPO_ROOT/.node-version" ]; then
    wanted=$(tr -d ' \t\r\n' <"$REPO_ROOT/.node-version")
    wanted=${wanted#v}
    if [ -n "$wanted" ]; then
        NODE_VERSION=$wanted
        node_source=".node-version"
    fi
fi
case "$NODE_VERSION" in '' | *[!0-9.]*) die "unexpected Node version in .node-version: $NODE_VERSION" ;; esac

# ------------------------------------------------------------------------------------------
# What is there already
# ------------------------------------------------------------------------------------------
PATH="$CARGO_BIN:$PATH"
export PATH

have_rustup=0
[ -x "$CARGO_BIN/rustup" ] && have_rustup=1
have_toolchain=0
have_target=0
have_xwin=0
if [ "$have_rustup" = 1 ]; then
    rustup toolchain list 2>/dev/null | grep -q "^$RUST_TOOLCHAIN-" && have_toolchain=1
    if [ "$have_toolchain" = 1 ]; then
        rustup target list --installed --toolchain "$RUST_TOOLCHAIN" 2>/dev/null | grep -qx "$WINDOWS_TARGET" && have_target=1
    fi
    [ -x "$CARGO_BIN/cargo-xwin" ] && "$CARGO_BIN/cargo-xwin" --version 2>/dev/null | grep -q " $CARGO_XWIN_VERSION\$" && have_xwin=1
fi
have_llvm=0
"$BREW" list --versions llvm >/dev/null 2>&1 && have_llvm=1
have_fnm=0
"$BREW" list --versions fnm >/dev/null 2>&1 && have_fnm=1
have_node=0
have_pnpm=0
FNM=''
if [ "$have_fnm" = 1 ]; then
    FNM="$("$BREW" --prefix)/bin/fnm"
    [ -x "$FNM" ] || FNM=$(command -v fnm)
    if [ -n "$FNM" ] && "$FNM" list 2>/dev/null | grep -Eq "v$NODE_VERSION([[:space:]]|\$)"; then
        have_node=1
        [ "$("$FNM" exec --using="$NODE_VERSION" pnpm --version 2>/dev/null)" = "$PNPM_VERSION" ] && have_pnpm=1
    fi
fi

status() { if [ "$1" = 1 ]; then printf 'done   '; else printf 'to do  '; fi; }
say ""
say "Plan (Mac, $HOST_TRIPLE). Sizes are rough estimates."
say "  $(status $have_rustup)rustup $RUSTUP_VERSION from static.rust-lang.org, SHA-256 checked; adds ~/.cargo/bin to"
say "         your shell's PATH (rustup's standard change to ~/.zshenv, ~/.bash_profile, ~/.profile)"
say "  $(status $have_toolchain)Rust $RUST_TOOLCHAIN (from $toolchain_source), minimal profile + rustfmt + clippy (~1.5 GB)"
say "  $(status $have_target)Rust target $WINDOWS_TARGET (~0.3 GB)"
say "  $(status $have_llvm)Homebrew llvm (~2 GB): clang-cl, llvm-lib, llvm-rc, libclang"
say "  $(status $have_fnm)Homebrew fnm (Node version manager; no shell changes)"
say "  $(status $have_node)Node.js $NODE_VERSION through fnm (from $node_source; ~0.2 GB), for this repo only"
say "  $(status $have_pnpm)pnpm $PNPM_VERSION through that Node's corepack"
say "  $(status $have_xwin)cargo-xwin $CARGO_XWIN_VERSION (built with cargo install --locked) and Microsoft's CRT +"
say "         Windows SDK files for x86_64 (~1.5 GB), only if you accept Microsoft's licence below"
say ""

if [ "$DRY_RUN" = 1 ]; then
    say "dry run: nothing was installed"
    exit 0
fi

if [ "$ASSUME_YES" != 1 ]; then
    [ -t 0 ] || die "no terminal to ask; run it in Terminal, or pass --yes after the owner agreed"
    printf 'Install the steps marked "to do"? Type yes to continue: '
    read -r answer
    [ "$answer" = yes ] || die "nothing installed"
fi

if [ "$have_xwin" != 1 ] && [ "$ACCEPT_LICENSE" != 1 ]; then
    say ""
    say "cargo-xwin downloads Microsoft's C runtime and Windows SDK files so the Mac can check Windows"
    say "code. Its README says: \"By using this software you are consented to accept the license at"
    say "$MS_LICENSE_URL\" (it redirects to Microsoft's Visual Studio licence terms)."
    if [ -t 0 ] && [ "$ASSUME_YES" != 1 ]; then
        printf 'Read that licence. Type "I accept" to install cargo-xwin and the SDK files, or press Enter to skip: '
        read -r answer
        [ "$answer" = "I accept" ] && ACCEPT_LICENSE=1
    fi
    [ "$ACCEPT_LICENSE" = 1 ] || say "skipping cargo-xwin and the SDK files (run again with --accept-microsoft-license once accepted)"
fi

# ------------------------------------------------------------------------------------------
# Install
# ------------------------------------------------------------------------------------------
WORK=$(mktemp -d "$TMP_BASE/kairo-setup.XXXXXX") || die "mktemp failed"

if [ "$have_rustup" != 1 ]; then
    step "rustup $RUSTUP_VERSION"
    url="https://static.rust-lang.org/rustup/archive/$RUSTUP_VERSION/$HOST_TRIPLE/rustup-init"
    curl --proto '=https' --tlsv1.2 -fsSL -A "$USER_AGENT" -o "$WORK/rustup-init" "$url" || die "download failed: $url"
    got=$(/usr/bin/shasum -a 256 "$WORK/rustup-init" | awk '{print $1}')
    [ "$got" = "$RUSTUP_SHA256" ] || die "rustup-init has the wrong SHA-256 ($got, expected $RUSTUP_SHA256); not running it"
    chmod +x "$WORK/rustup-init"
    "$WORK/rustup-init" -y --default-toolchain none --profile minimal || die "rustup-init failed"
fi

if [ "$have_toolchain" != 1 ]; then
    step "Rust $RUST_TOOLCHAIN"
    if [ -f "$REPO_ROOT/rust-toolchain.toml" ]; then
        (cd "$REPO_ROOT" && rustup toolchain install) || die "rustup toolchain install failed"
    else
        rustup toolchain install "$RUST_TOOLCHAIN" --profile minimal --component rustfmt --component clippy || die "rustup toolchain install failed"
    fi
fi

if [ "$have_target" != 1 ]; then
    step "Rust target $WINDOWS_TARGET"
    rustup target add --toolchain "$RUST_TOOLCHAIN" "$WINDOWS_TARGET" || die "rustup target add failed"
fi

if [ "$have_llvm" != 1 ]; then
    step "Homebrew llvm"
    "$BREW" install llvm || die "brew install llvm failed"
fi

if [ "$have_fnm" != 1 ]; then
    step "Homebrew fnm"
    "$BREW" install fnm || die "brew install fnm failed"
    FNM="$("$BREW" --prefix)/bin/fnm"
    [ -x "$FNM" ] || FNM=$(command -v fnm)
fi
if [ -z "$FNM" ] || [ ! -x "$FNM" ]; then
    die "fnm is not on PATH after installing it"
fi

if [ "$have_node" != 1 ]; then
    step "Node.js $NODE_VERSION (fnm)"
    had_default=0
    "$FNM" default >/dev/null 2>&1 && had_default=1
    "$FNM" install "$NODE_VERSION" || die "fnm install $NODE_VERSION failed"
    # fnm makes its first Node the "default" of its shell integration. If it had none before, keep
    # the global Node as the default instead, in case that integration is ever switched on.
    if [ "$had_default" != 1 ]; then
        "$FNM" default system >/dev/null 2>&1 || true
    fi
fi

if [ "$have_pnpm" != 1 ]; then
    step "pnpm $PNPM_VERSION (corepack of Node $NODE_VERSION)"
    COREPACK_ENABLE_DOWNLOAD_PROMPT=0 "$FNM" exec --using="$NODE_VERSION" corepack install -g "pnpm@$PNPM_VERSION" ||
        die "corepack install -g pnpm@$PNPM_VERSION failed"
    "$FNM" exec --using="$NODE_VERSION" corepack enable pnpm || die "corepack enable pnpm failed"
fi

if [ "$have_xwin" != 1 ] && [ "$ACCEPT_LICENSE" = 1 ]; then
    step "cargo-xwin $CARGO_XWIN_VERSION"
    cargo "+$RUST_TOOLCHAIN" install --locked cargo-xwin --version "$CARGO_XWIN_VERSION" || die "cargo install cargo-xwin failed"
    step "Microsoft CRT and Windows SDK files for x86_64 (cargo xwin cache xwin)"
    XWIN_ARCH=x86_64 cargo "+$RUST_TOOLCHAIN" xwin cache xwin || die "cargo xwin cache xwin failed"
fi

# ------------------------------------------------------------------------------------------
# Summary
# ------------------------------------------------------------------------------------------
step "Done"
say "rustc:      $(rustc "+$RUST_TOOLCHAIN" --version 2>/dev/null || echo 'not found')"
say "cargo-xwin: $(cargo-xwin --version 2>/dev/null || echo 'not installed')"
say "llvm:       $("$BREW" list --versions llvm 2>/dev/null || echo 'not found')"
say "node:       $("$FNM" exec --using="$NODE_VERSION" node --version 2>/dev/null || echo 'not found') (fnm, this repo only)"
say "pnpm:       $("$FNM" exec --using="$NODE_VERSION" pnpm --version 2>/dev/null || echo 'not found')"
LLVM_PREFIX=$("$BREW" --prefix llvm 2>/dev/null)
cat <<EOF

Next:
  - Open a new terminal so that ~/.cargo/bin is on PATH.
  - For Windows checks on this 8 GB Mac (docs/PLAN.md section 3.1):
      export XWIN_ARCH=x86_64 CARGO_BUILD_JOBS=4
      export PATH="$LLVM_PREFIX/bin:\$PATH" LIBCLANG_PATH="$LLVM_PREFIX/lib"
      cargo xwin check --target $WINDOWS_TARGET --workspace
  - Node tools for this repo run through fnm without touching your global Node, e.g.:
      fnm exec --using=$NODE_VERSION pnpm --version
EOF
