#!/bin/sh
set -eu

temporary_root=${TMPDIR:-/tmp}
temporary_directory=$(mktemp -d "$temporary_root/quiverdl-installer-test.XXXXXX")
cleanup() {
  case "$temporary_directory" in
    "$temporary_root"/quiverdl-installer-test.*) rm -rf -- "$temporary_directory" ;;
    *) echo "refusing to remove unexpected test directory" >&2; exit 1 ;;
  esac
}
trap cleanup EXIT HUP INT TERM

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
if [ "${OS:-}" = "Windows_NT" ]; then
  host_path="$temporary_directory/quiver&host"
else
  host_path="$temporary_directory/quiver\"\\host"
fi
printf '%s\n' '#!/bin/sh' 'exit 0' > "$host_path"
chmod 700 "$host_path"
extension_id=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa

HOME="$temporary_directory/linux-home" \
XDG_CONFIG_HOME="$temporary_directory/linux-config" \
  "$script_dir/install-linux.sh" "$host_path" "$extension_id"
linux_manifest="$temporary_directory/linux-config/google-chrome/NativeMessagingHosts/app.quiverdl.native.json"

HOME="$temporary_directory/macos-home" \
  "$script_dir/install-macos.sh" "$host_path" "$extension_id"
macos_manifest="$temporary_directory/macos-home/Library/Application Support/Google/Chrome/NativeMessagingHosts/app.quiverdl.native.json"

if [ "${OS:-}" = "Windows_NT" ]; then
  linux_manifest=$(cygpath -w "$linux_manifest")
  macos_manifest=$(cygpath -w "$macos_manifest")
fi

EXPECTED_HOST="$host_path" node -e '
  const fs = require("fs");
  const expected = process.env.EXPECTED_HOST;
  for (const path of process.argv.slice(1)) {
    const manifest = JSON.parse(fs.readFileSync(path, "utf8"));
    if (process.platform !== "win32" && manifest.path !== expected) {
      throw new Error(`Incorrect encoded path in ${path}`);
    }
  }
' "$linux_manifest" "$macos_manifest"
