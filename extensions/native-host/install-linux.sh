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

if printf '%s' "$host_path" | LC_ALL=C grep -q '[[:cntrl:]]'; then
  echo "host path cannot contain control characters" >&2
  exit 2
fi
json_host=$(printf '%s' "$host_path" | sed -e 's/\\/\\\\/g' -e 's/"/\\"/g')

chrome_dir="${XDG_CONFIG_HOME:-$HOME/.config}/google-chrome/NativeMessagingHosts"
chromium_dir="${XDG_CONFIG_HOME:-$HOME/.config}/chromium/NativeMessagingHosts"
firefox_dir="$HOME/.mozilla/native-messaging-hosts"
mkdir -p "$chrome_dir" "$chromium_dir" "$firefox_dir"

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
write_chromium_manifest "$chromium_dir/app.quiverdl.native.json"
write_firefox_manifest "$firefox_dir/app.quiverdl.native.json"
chmod 600 "$chrome_dir/app.quiverdl.native.json" "$chromium_dir/app.quiverdl.native.json" "$firefox_dir/app.quiverdl.native.json"
echo "Installed QuiverDL native messaging manifests."
