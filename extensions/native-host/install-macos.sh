#!/bin/sh
set -eu

if [ "$#" -ne 2 ]; then
  echo "usage: $0 /absolute/path/to/quiver-native-host CHROMIUM_EXTENSION_ID" >&2
  exit 2
fi

host_path=$1
extension_id=$2
case "$host_path" in /*) ;; *) echo "host path must be absolute" >&2; exit 2;; esac
if [ ! -f "$host_path" ] || [ ! -x "$host_path" ]; then
  echo "native host must exist and be executable: $host_path" >&2
  exit 2
fi
case "$extension_id" in
  ''|*[!a-p]*) echo "Chromium extension ID must contain 32 lowercase letters from a through p" >&2; exit 2;;
esac
if [ "${#extension_id}" -ne 32 ]; then
  echo "Chromium extension ID must contain 32 lowercase letters from a through p" >&2
  exit 2
fi

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
escaped_host=$(printf '%s' "$host_path" | sed 's/[&|\\]/\\&/g')
chrome_dir="$HOME/Library/Application Support/Google/Chrome/NativeMessagingHosts"
chrome_testing_dir="$HOME/Library/Application Support/Google/ChromeForTesting/NativeMessagingHosts"
chromium_dir="$HOME/Library/Application Support/Chromium/NativeMessagingHosts"
firefox_dir="$HOME/Library/Application Support/Mozilla/NativeMessagingHosts"
mkdir -p "$chrome_dir" "$chrome_testing_dir" "$chromium_dir" "$firefox_dir"

chromium_manifest=$(sed \
  -e "s|REPLACE_WITH_ABSOLUTE_NATIVE_HOST_PATH|$escaped_host|" \
  -e "s|REPLACE_WITH_EXTENSION_ID|$extension_id|" \
  "$script_dir/chromium-host.json")
printf '%s\n' "$chromium_manifest" > "$chrome_dir/app.quiverdl.native.json"
printf '%s\n' "$chromium_manifest" > "$chrome_testing_dir/app.quiverdl.native.json"
printf '%s\n' "$chromium_manifest" > "$chromium_dir/app.quiverdl.native.json"
sed "s|REPLACE_WITH_ABSOLUTE_NATIVE_HOST_PATH|$escaped_host|" \
  "$script_dir/firefox-host.json" > "$firefox_dir/app.quiverdl.native.json"
chmod 600 \
  "$chrome_dir/app.quiverdl.native.json" \
  "$chrome_testing_dir/app.quiverdl.native.json" \
  "$chromium_dir/app.quiverdl.native.json" \
  "$firefox_dir/app.quiverdl.native.json"

echo "Installed QuiverDL native messaging manifests for Chrome, Chrome for Testing, Chromium, and Firefox."
