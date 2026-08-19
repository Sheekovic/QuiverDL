#!/usr/bin/env sh
set -eu

if [ "$#" -ne 2 ]; then
  echo "usage: $0 /absolute/path/to/quiver-native-host CHROMIUM_EXTENSION_ID" >&2
  exit 2
fi

host_path=$1
extension_id=$2
case "$host_path" in /*) ;; *) echo "host path must be absolute" >&2; exit 2;; esac

chrome_dir="${XDG_CONFIG_HOME:-$HOME/.config}/google-chrome/NativeMessagingHosts"
chromium_dir="${XDG_CONFIG_HOME:-$HOME/.config}/chromium/NativeMessagingHosts"
firefox_dir="$HOME/.mozilla/native-messaging-hosts"
mkdir -p "$chrome_dir" "$chromium_dir" "$firefox_dir"

chromium_manifest=$(sed -e "s|REPLACE_WITH_ABSOLUTE_NATIVE_HOST_PATH|$host_path|" -e "s|REPLACE_WITH_EXTENSION_ID|$extension_id|" "$(dirname "$0")/chromium-host.json")
printf '%s\n' "$chromium_manifest" > "$chrome_dir/app.quiverdl.native.json"
printf '%s\n' "$chromium_manifest" > "$chromium_dir/app.quiverdl.native.json"
sed "s|REPLACE_WITH_ABSOLUTE_NATIVE_HOST_PATH|$host_path|" "$(dirname "$0")/firefox-host.json" > "$firefox_dir/app.quiverdl.native.json"
chmod 600 "$chrome_dir/app.quiverdl.native.json" "$chromium_dir/app.quiverdl.native.json" "$firefox_dir/app.quiverdl.native.json"
echo "Installed QuiverDL native messaging manifests."
