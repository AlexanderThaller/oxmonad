#!/bin/sh
# Build oxmonad with the deploy profile and install it, together with the
# LightDM session wrapper and greeter entry (see contrib/lightdm/README.adoc).
#
# Run as your normal user; only the install steps use sudo. Safe while
# oxmonad is running: `install` replaces the file instead of writing into it,
# and a running oxmonad of this session is then restarted into the new
# binary (`oxmonad --restart`, like Mod+q; windows stay where they are).
#
# Usage: deploy/install.sh [PREFIX]   (default PREFIX: /usr/local)
set -eu

prefix=${1:-/usr/local}
repo=$(cd "$(dirname "$0")/.." && pwd)
cd "$repo"

if ! command -v river >/dev/null 2>&1; then
    echo "warning: river is not installed (sudo pacman -S river)" >&2
fi

cargo build --profile deploy -p oxmonad
./target/deploy/oxmonad --check-config

# The session wrapper hardcodes /usr/local/bin/oxmonad; point it at PREFIX.
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
sed "s|/usr/local/bin/oxmonad|$prefix/bin/oxmonad|" contrib/lightdm/oxmonad-session >"$tmp/oxmonad-session"
sed "s|/usr/local/bin/oxmonad-session|$prefix/bin/oxmonad-session|" contrib/lightdm/oxmonad.desktop >"$tmp/oxmonad.desktop"

sudo install -Dm755 target/deploy/oxmonad "$prefix/bin/oxmonad"
sudo install -Dm755 "$tmp/oxmonad-session" "$prefix/bin/oxmonad-session"
sudo install -Dm755 contrib/oxmonad-lock "$prefix/bin/oxmonad-lock"
sudo install -Dm644 "$tmp/oxmonad.desktop" /usr/share/wayland-sessions/oxmonad.desktop

echo "installed oxmonad to $prefix/bin"

if ! "$prefix/bin/oxmonad" --restart 2>/dev/null; then
    echo "no running oxmonad in this session; pick 'oxmonad' in LightDM to start it"
fi
