#!/usr/bin/env sh
set -eu

if [ "$#" -ne 2 ]; then
  echo "usage: $0 /absolute/path/to/quiver-native-host CHROMIUM_EXTENSION_ID" >&2
  exit 2
fi

host_path=$1
extension_id=$2
case "$host_path" in /*) ;; *) echo "host path must be absolute" >&2; exit 2;; esac
case "$extension_id" in
  ''|*[!a-p]*) echo "Chromium extension ID must contain 32 lowercase letters from a through p" >&2; exit 2;;
esac
if [ "${#extension_id}" -ne 32 ]; then
  echo "Chromium extension ID must contain 32 lowercase letters from a through p" >&2
  exit 2
fi

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
escaped_host=$(printf '%s' "$host_path" | sed 's/[&|\\]/\\&/g')

chrome_dir="${XDG_CONFIG_HOME:-$HOME/.config}/google-chrome/NativeMessagingHosts"
chromium_dir="${XDG_CONFIG_HOME:-$HOME/.config}/chromium/NativeMessagingHosts"
firefox_dir="$HOME/.mozilla/native-messaging-hosts"
mkdir -p "$chrome_dir" "$chromium_dir" "$firefox_dir"

chromium_manifest=$(sed -e "s|REPLACE_WITH_ABSOLUTE_NATIVE_HOST_PATH|$escaped_host|" -e "s|REPLACE_WITH_EXTENSION_ID|$extension_id|" "$script_dir/chromium-host.json")
printf '%s\n' "$chromium_manifest" > "$chrome_dir/app.quiverdl.native.json"
printf '%s\n' "$chromium_manifest" > "$chromium_dir/app.quiverdl.native.json"
sed "s|REPLACE_WITH_ABSOLUTE_NATIVE_HOST_PATH|$escaped_host|" "$script_dir/firefox-host.json" > "$firefox_dir/app.quiverdl.native.json"
chmod 600 "$chrome_dir/app.quiverdl.native.json" "$chromium_dir/app.quiverdl.native.json" "$firefox_dir/app.quiverdl.native.json"
echo "Installed QuiverDL native messaging manifests."
