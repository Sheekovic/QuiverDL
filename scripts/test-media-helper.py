"""Focused regression checks for the packaged yt-dlp bridge."""

from __future__ import annotations

import importlib.util
from pathlib import Path


HELPER_PATH = (
    Path(__file__).resolve().parents[1]
    / "apps"
    / "desktop"
    / "src-tauri"
    / "python"
    / "quiver_media.py"
)

spec = importlib.util.spec_from_file_location("quiver_media", HELPER_PATH)
if spec is None or spec.loader is None:
    raise RuntimeError("Could not load the QuiverDL media helper")
quiver_media = importlib.util.module_from_spec(spec)
spec.loader.exec_module(quiver_media)


unsupported = RuntimeError(
    "ERROR: [Liability] This website is not supported and will not be supported.\n"
    "DO NOT open issues for it"
)
assert quiver_media.safe_error(unsupported) == (
    "This media website is not supported by yt-dlp. "
    "Try a direct media URL or another supported website."
)

multiline = RuntimeError("ERROR: The useful explanation\nA less useful trailing line")
assert quiver_media.safe_error(multiline) == "The useful explanation"

secret_url = RuntimeError("ERROR: Unable to inspect https://example.test/private?id=secret")
assert quiver_media.safe_error(secret_url) == "Unable to inspect [media URL]"

print("Media helper regression checks passed.")
