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

if printf '%s' "$host_path" | LC_ALL=C grep -q '[[:cntrl:]]'; then
  echo "host path cannot contain control characters" >&2
  exit 2
fi
json_host=$(printf '%s' "$host_path" | sed -e 's/\\/\\\\/g' -e 's/"/\\"/g')
chrome_dir="$HOME/Library/Application Support/Google/Chrome/NativeMessagingHosts"
chrome_testing_dir="$HOME/Library/Application Support/Google/ChromeForTesting/NativeMessagingHosts"
chromium_dir="$HOME/Library/Application Support/Chromium/NativeMessagingHosts"
firefox_dir="$HOME/Library/Application Support/Mozilla/NativeMessagingHosts"
mkdir -p "$chrome_dir" "$chrome_testing_dir" "$chromium_dir" "$firefox_dir"

write_chromium_manifest() {
  target=$1
  {
    printf '%s\n' '{'
    printf '%s\n' '  "name": "app.quiverdl.native",'
    printf '%s\n' '  "description": "QuiverDL authenticated native messaging bridge",'
    printf '  "path": "%s",\n' "$json_host"
    printf '%s\n' '  "type": "stdio",'
    printf '  "allowed_origins": ["chrome-extension://%s/"]\n' "$extension_id"
    printf '%s\n' '}'
  } > "$target"
}

write_firefox_manifest() {
  target=$1
  {
    printf '%s\n' '{'
    printf '%s\n' '  "name": "app.quiverdl.native",'
    printf '%s\n' '  "description": "QuiverDL authenticated native messaging bridge",'
    printf '  "path": "%s",\n' "$json_host"
    printf '%s\n' '  "type": "stdio",'
    printf '%s\n' '  "allowed_extensions": ["quiverdl@quiverdl.app"]'
    printf '%s\n' '}'
  } > "$target"
}

write_chromium_manifest "$chrome_dir/app.quiverdl.native.json"
write_chromium_manifest "$chrome_testing_dir/app.quiverdl.native.json"
write_chromium_manifest "$chromium_dir/app.quiverdl.native.json"
write_firefox_manifest "$firefox_dir/app.quiverdl.native.json"
chmod 600 \
  "$chrome_dir/app.quiverdl.native.json" \
  "$chrome_testing_dir/app.quiverdl.native.json" \
  "$chromium_dir/app.quiverdl.native.json" \
  "$firefox_dir/app.quiverdl.native.json"

echo "Installed QuiverDL native messaging manifests for Chrome, Chrome for Testing, Chromium, and Firefox."
