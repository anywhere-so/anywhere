#!/bin/sh
# Installs the anywhere CLI, then its skill for your AI agents:
#
#   curl -fsSL https://anywhere.so/install.sh | sh
#
# It downloads the CLI built for this computer, and its SHA-256, from the
# Anywhere download service, checks the
# one against the other, installs the CLI as ~/.local/bin/anywhere, and runs
# `anywhere skill install`. Settings, each optional:
#
#   ANYWHERE_INSTALL_BASE  the platform's origin, https://anywhere.so unless
#                          set, for testing against another installation
#   ANYWHERE_INSTALL_DIR   the folder to install into, ~/.local/bin unless set
#
# Everything is inside main, called on the last line, so a download cut off
# part way runs nothing.

set -eu

fail() {
  printf 'anywhere install: error: %s\n' "$*" >&2
  exit 1
}

say() {
  printf 'anywhere install: %s\n' "$*"
}

# The minimum macOS version supported by the distributed CLI.
MIN_MACOS=13

# The CLI's target for this computer: those the release bundle carries.
target() {
  os=$(uname -s) || fail "uname couldn't name this system"
  arch=$(uname -m) || fail "uname couldn't name this machine's architecture"
  case "$os/$arch" in
    Darwin/arm64 | Darwin/x86_64) check_macos ;;
  esac
  case "$os/$arch" in
    Darwin/arm64) echo aarch64-apple-darwin ;;
    Darwin/x86_64) echo x86_64-apple-darwin ;;
    Linux/x86_64) echo x86_64-unknown-linux-musl ;;
    *) fail "there's no anywhere CLI for $os on $arch yet: it's built for macOS $MIN_MACOS or later on Apple silicon or Intel, and Linux on x86_64" ;;
  esac
}

# Refuses a macOS older than the CLI runs on.
check_macos() {
  version=$(sw_vers -productVersion) || fail "sw_vers couldn't name this Mac's macOS version"
  major=${version%%.*}
  case $major in
    '' | *[!0-9]*) fail "sw_vers named this Mac's macOS version $version, which isn't a version" ;;
  esac
  [ "$major" -ge "$MIN_MACOS" ] ||
    fail "the anywhere CLI needs macOS $MIN_MACOS or later, and this Mac runs macOS $version; update macOS, then run this again"
}

# The platform's origin: https://, or http:// on this machine only, with a
# host, an optional port, and no path.
check_base() {
  case $1 in
    https://*) rest=${1#https://} ;;
    http://127.0.0.1 | http://127.0.0.1:* | http://localhost | http://localhost:* | http://\[::1\] | http://\[::1\]:*)
      rest=${1#http://} ;;
    *) fail "ANYWHERE_INSTALL_BASE is $1, but it must be an https:// origin, such as https://anywhere.so" ;;
  esac
  case $rest in
    '' | *[!]A-Za-z0-9.:[-]*) fail "ANYWHERE_INSTALL_BASE is $1, but it must be an origin alone, such as https://anywhere.so, with no path, query or trailing slash" ;;
  esac
}

# Downloads $1 into the file $2, and fails unless the platform answers 200.
download() {
  case $1 in
    https://*) proto='=https' ;;
    *) proto='=http' ;;
  esac
  # No redirects, so the files come only from the origin asked; deadlines, so
  # a stalled or endless download fails rather than hangs, an hour being far
  # beyond any build's; and retries only of what curl knows is temporary,
  # since a download is safe to repeat.
  status=$(curl --silent --show-error --proto "$proto" --output "$2" --write-out '%{http_code}' \
    --connect-timeout 30 --speed-limit 1024 --speed-time 60 --max-time 3600 --retry 3 "$1") ||
    fail "couldn't download $1, as curl says above"
  case $status in
    200) ;;
    404) fail "$1 isn't there (HTTP 404): the platform has no CLI for this computer" ;;
    *) fail "downloading $1 answered HTTP $status, not 200; try again later" ;;
  esac
}

main() {
  base=${ANYWHERE_INSTALL_BASE:-https://anywhere.so}
  check_base "$base"
  if [ -n "${ANYWHERE_INSTALL_DIR:-}" ]; then
    dir=$ANYWHERE_INSTALL_DIR
  elif [ -n "${HOME:-}" ]; then
    dir=$HOME/.local/bin
  else
    fail "HOME isn't set, so there's no ~/.local/bin; set ANYWHERE_INSTALL_DIR to the folder to install into"
  fi
  case $dir in
    /*) ;;
    *) fail "ANYWHERE_INSTALL_DIR is $dir, but it must be an absolute path" ;;
  esac
  command -v curl >/dev/null || fail "curl isn't installed; install it, then run this again"
  if command -v sha256sum >/dev/null; then
    sha256() { sha256sum "$1"; }
  elif command -v shasum >/dev/null; then
    sha256() { shasum -a 256 "$1"; }
  else
    fail "neither sha256sum nor shasum is installed, so the download can't be checked; install either, then run this again"
  fi
  target=$(target)
  url=$base/cli/$target/anywhere

  mkdir -p "$dir" || fail "couldn't make the folder $dir"
  [ ! -d "$dir/anywhere" ] || fail "$dir/anywhere is a folder; move it, then run this again"
  # Downloaded beside where it goes, so moving it there is one rename, and a
  # CLI that's there already is replaced whole or not at all.
  work=$(mktemp -d "$dir/.anywhere-install.XXXXXX") || fail "couldn't make a temporary folder in $dir"
  trap 'rm -rf "$work"' EXIT
  trap 'exit 1' HUP INT TERM

  say "downloading the CLI for $target from $base"
  download "$url" "$work/anywhere"
  download "$url.sha256" "$work/anywhere.sha256"
  expected='' name=''
  # read says no for a last line without a newline, though it read the line.
  read -r expected name <"$work/anywhere.sha256" || [ -n "$expected" ] || fail "$url.sha256 is empty"
  case $expected in
    *[!0-9a-f]*) expected= ;;
  esac
  if [ "${#expected}" -ne 64 ] || [ "$name" != anywhere ] ||
    [ "$(cat "$work/anywhere.sha256")" != "$expected  anywhere" ]; then
    fail "$url.sha256 isn't a SHA-256 and the name anywhere; nothing was installed"
  fi
  actual=$(sha256 "$work/anywhere") || fail "couldn't hash the download"
  actual=${actual%% *}
  [ "$actual" = "$expected" ] ||
    fail "the download's SHA-256 is $actual, but $url.sha256 says $expected; nothing was installed. Run this again, since the download may have changed on the way"
  chmod 0755 "$work/anywhere" || fail "couldn't make the download executable"
  # Their standard input is the terminal's or nothing, never the rest of a
  # script piped into sh.
  version=$("$work/anywhere" --version </dev/null) ||
    fail "the downloaded CLI doesn't run on this computer, as it says above; nothing was installed"
  mv -f "$work/anywhere" "$dir/anywhere" || fail "couldn't move the CLI into $dir"
  say "installed $version as $dir/anywhere"

  say "installing the anywhere skill for your AI agents"
  "$dir/anywhere" skill install </dev/null ||
    fail "\`anywhere skill install\` failed, as it says above, so your AI agents don't have the skill. The CLI is installed as $dir/anywhere; run \`$dir/anywhere skill install\` again once that's fixed"

  # What `anywhere` runs by its name: this CLI, another found first, or none.
  found=$(command -v anywhere) || found=''
  command=anywhere
  if [ "$found" != "$dir/anywhere" ]; then
    command=$dir/anywhere
    case ":${PATH:-}:" in
      *":$dir:"*) ;;
      *)
        say "$dir isn't on your PATH. To run anywhere by its name, add this line to your shell's startup file, such as ~/.zshrc or ~/.bashrc:"
        printf '\n  export PATH="%s:%s"\n\n' "$dir" "\$PATH"
        ;;
    esac
    if [ -n "$found" ]; then
      say "warning: another anywhere, $found, comes first on your PATH, so \`anywhere\` runs it, not this one; remove it, or put $dir before its folder on your PATH"
    fi
  fi
  say "next, sign in: $command login"
}

main "$@"
