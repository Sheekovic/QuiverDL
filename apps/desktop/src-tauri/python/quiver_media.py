"""Minimal JSON-lines bridge between QuiverDL and the yt-dlp Python API."""

from __future__ import annotations

import base64
import json
import os
import re
import sys
from pathlib import Path
from typing import Any, Optional
from urllib.parse import urlsplit


MAX_THUMBNAIL_BYTES = 384 * 1024
SAFE_THUMBNAIL_TYPES = {"image/jpeg", "image/png", "image/webp", "image/gif"}


def emit(payload: dict[str, Any]) -> None:
    sys.stdout.write(json.dumps(payload, ensure_ascii=False, separators=(",", ":")) + "\n")
    sys.stdout.flush()


def safe_error(error: BaseException) -> str:
    message = str(error).strip().splitlines()[-1] if str(error).strip() else error.__class__.__name__
    message = re.sub(r"(?:https?|ftp)://\S+", "[media URL]", message, flags=re.IGNORECASE)
    return message[:600]


def load_request() -> dict[str, Any]:
    line = sys.stdin.readline(1024 * 1024)
    if not line:
        raise ValueError("QuiverDL did not provide a media request")
    request = json.loads(line)
    if not isinstance(request, dict):
        raise ValueError("The media request is invalid")
    return request


def format_metadata(info: dict[str, Any], thumbnail: Optional[str] = None) -> dict[str, Any]:
    formats: list[dict[str, Any]] = []
    seen: set[tuple[Any, ...]] = set()
    for item in info.get("formats") or []:
        if not isinstance(item, dict) or not item.get("format_id"):
            continue
        video = item.get("vcodec") not in (None, "none")
        audio = item.get("acodec") not in (None, "none")
        if not video and not audio:
            continue
        height = item.get("height") if isinstance(item.get("height"), int) else None
        size = item.get("filesize") or item.get("filesize_approx")
        size = size if isinstance(size, int) and size >= 0 else None
        extension = str(item.get("ext") or "unknown")[:16]
        key = (height, extension, video, audio, size)
        if key in seen:
            continue
        seen.add(key)
        if video:
            label = f"{height}p" if height else str(item.get("resolution") or "Video")
            if audio:
                label += " · video + audio"
        else:
            abr = item.get("abr")
            label = f"Audio{f' · {round(abr)} kbps' if isinstance(abr, (int, float)) else ''}"
        formats.append(
            {
                "formatId": str(item["format_id"])[:128],
                "label": label[:160],
                "height": height,
                "extension": extension,
                "audioOnly": audio and not video,
                "hasAudio": audio,
                "approxBytes": str(size) if size is not None else None,
            }
        )
    formats.sort(key=lambda item: (item["audioOnly"], -(item["height"] or 0), item["extension"]))
    duration = info.get("duration")
    return {
        "title": str(info.get("title") or "Untitled media")[:240],
        "extractor": str(info.get("extractor_key") or info.get("extractor") or "yt-dlp")[:80],
        "thumbnail": thumbnail,
        "durationSeconds": int(duration) if isinstance(duration, (int, float)) and duration >= 0 else None,
        "formats": formats[:160],
    }


def quality_options(quality: str) -> tuple[str, list[dict[str, Any]]]:
    if quality == "best":
        return "bv*+ba/b", []
    if quality in {"2160", "1440", "1080", "720", "480", "360"}:
        return f"bv*[height<=?{quality}]+ba/b[height<=?{quality}]", []
    if quality == "audio-mp3":
        return "ba/b", [{"key": "FFmpegExtractAudio", "preferredcodec": "mp3", "preferredquality": "0"}]
    if quality == "audio-m4a":
        return "ba[ext=m4a]/ba/b", [{"key": "FFmpegExtractAudio", "preferredcodec": "m4a"}]
    if quality.startswith("format:") and 7 < len(quality) <= 135:
        format_id = quality[7:]
        if not re.fullmatch(r"[A-Za-z0-9._+-]+", format_id):
            raise ValueError("The selected media format identifier is invalid")
        return format_id, []
    raise ValueError("The selected media quality is unsupported")


def apply_proxy(options: dict[str, Any], request: dict[str, Any]) -> None:
    proxy = request.get("proxy")
    if proxy is not None:
        if not isinstance(proxy, str) or len(proxy) > 16 * 1024:
            raise ValueError("The media proxy configuration is invalid")
        options["proxy"] = proxy


def configure_proxy_bypass(ydl: Any, request: dict[str, Any]) -> None:
    bypass = request.get("proxyBypass")
    if bypass is None:
        return
    if not isinstance(bypass, str) or len(bypass) > 8 * 1024 or any(ord(char) < 32 for char in bypass):
        raise ValueError("The media proxy bypass list is invalid")
    ydl.proxies["no"] = bypass


def fetch_thumbnail(ydl: Any, value: Any) -> Optional[str]:
    if not isinstance(value, str) or len(value) > 4096:
        return None
    try:
        parsed = urlsplit(value)
    except ValueError:
        return None
    if parsed.scheme != "https" or not parsed.hostname or parsed.username or parsed.password:
        return None
    try:
        with ydl.urlopen(value) as response:
            content_type = str(response.headers.get("Content-Type") or "").split(";", 1)[0].lower()
            if content_type not in SAFE_THUMBNAIL_TYPES:
                return None
            content_length = response.headers.get("Content-Length")
            if content_length is not None and int(content_length) > MAX_THUMBNAIL_BYTES:
                return None
            data = response.read(MAX_THUMBNAIL_BYTES + 1)
    except Exception:
        return None
    if len(data) > MAX_THUMBNAIL_BYTES:
        return None
    encoded = base64.b64encode(data).decode("ascii")
    return f"data:{content_type};base64,{encoded}"


def inspect_media(ydl_class: Any, request: dict[str, Any], url: str) -> None:
    options = {
        "quiet": True,
        "no_warnings": True,
        "noplaylist": True,
        "socket_timeout": 30,
        "skip_download": True,
    }
    apply_proxy(options, request)
    with ydl_class(options) as ydl:
        configure_proxy_bypass(ydl, request)
        info = ydl.extract_info(url, download=False)
        thumbnail = fetch_thumbnail(ydl, info.get("thumbnail") if isinstance(info, dict) else None)
        sanitized = ydl.sanitize_info(info)
    emit({"type": "metadata", "metadata": format_metadata(sanitized, thumbnail)})


def detect_media(url: str) -> None:
    from yt_dlp.extractor import gen_extractors

    supported = any(
        extractor.IE_NAME != "generic" and extractor.suitable(url)
        for extractor in gen_extractors()
    )
    emit({"type": "detection", "supported": supported})


def download_media(ydl_class: Any, request: dict[str, Any], url: str) -> None:
    destination = Path(str(request.get("destinationDirectory") or "")).expanduser().resolve()
    if not destination.is_absolute():
        raise ValueError("The media destination must be absolute")
    destination.mkdir(parents=True, exist_ok=True)
    quality = str(request.get("quality") or "best")
    selector, postprocessors = quality_options(quality)
    reported_paths: list[str] = []
    last_progress: dict[str, Any] = {"downloaded": 0, "total": None}

    def progress_hook(data: dict[str, Any]) -> None:
        status = data.get("status")
        if status == "downloading":
            downloaded = data.get("downloaded_bytes")
            total = data.get("total_bytes") or data.get("total_bytes_estimate")
            last_progress["downloaded"] = downloaded if isinstance(downloaded, int) else 0
            last_progress["total"] = total if isinstance(total, int) else None
            emit(
                {
                    "type": "progress",
                    "status": "downloading",
                    "downloadedBytes": str(downloaded if isinstance(downloaded, int) else 0),
                    "totalBytes": str(total) if isinstance(total, int) else None,
                }
            )
        elif status == "finished":
            filename = data.get("filename")
            if isinstance(filename, str):
                reported_paths.append(filename)
            emit({
                "type": "progress",
                "status": "verifying",
                "downloadedBytes": str(last_progress["downloaded"]),
                "totalBytes": str(last_progress["total"]) if last_progress["total"] is not None else None,
            })

    def postprocessor_hook(data: dict[str, Any]) -> None:
        info = data.get("info_dict") or {}
        filepath = info.get("filepath") or data.get("filepath")
        if isinstance(filepath, str):
            reported_paths.append(filepath)
        if data.get("status") == "started":
            emit({
                "type": "progress",
                "status": "extracting",
                "downloadedBytes": str(last_progress["downloaded"]),
                "totalBytes": str(last_progress["total"]) if last_progress["total"] is not None else None,
            })

    options = {
        "quiet": True,
        "no_warnings": True,
        "noplaylist": True,
        "socket_timeout": 30,
        "retries": 3,
        "fragment_retries": 3,
        "concurrent_fragment_downloads": 4,
        "continuedl": True,
        "overwrites": False,
        "format": selector,
        "outtmpl": str(destination / "%(title).180B [%(id)s].%(ext)s"),
        "progress_hooks": [progress_hook],
        "postprocessor_hooks": [postprocessor_hook],
        "postprocessors": postprocessors,
    }
    apply_proxy(options, request)
    with ydl_class(options) as ydl:
        configure_proxy_bypass(ydl, request)
        info = ydl.extract_info(url, download=True)
        if isinstance(info, dict):
            for item in info.get("requested_downloads") or []:
                if isinstance(item, dict) and isinstance(item.get("filepath"), str):
                    reported_paths.append(item["filepath"])
            for key in ("filepath", "_filename"):
                if isinstance(info.get(key), str):
                    reported_paths.append(info[key])
            prepared = ydl.prepare_filename(info)
            if isinstance(prepared, str):
                reported_paths.append(prepared)

    resolved = None
    for reported in reversed(reported_paths):
        candidate = Path(reported).resolve()
        try:
            inside_destination = os.path.commonpath([str(destination), str(candidate)]) == str(destination)
        except ValueError:
            inside_destination = False
        if not inside_destination:
            raise RuntimeError("yt-dlp returned a file outside the selected destination")
        if candidate.is_file():
            resolved = candidate
            break
    if resolved is None:
        raise RuntimeError("yt-dlp completed without producing a media file")
    emit(
        {
            "type": "complete",
            "destination": str(resolved),
            "bytesWritten": str(resolved.stat().st_size),
        }
    )


def main() -> None:
    request = load_request()
    action = request.get("action")
    url = str(request.get("url") or "").strip()
    if len(url) > 8192 or not re.match(r"^https?://", url, re.IGNORECASE):
        raise ValueError("Only HTTP and HTTPS media URLs are supported")
    try:
        from yt_dlp import YoutubeDL
    except ImportError as error:
        raise RuntimeError("yt-dlp is not installed. Run: python -m pip install -U yt-dlp") from error

    if action == "detect":
        detect_media(url)
    elif action == "inspect":
        inspect_media(YoutubeDL, request, url)
    elif action == "download":
        download_media(YoutubeDL, request, url)
    else:
        raise ValueError("The media bridge action is unsupported")


if __name__ == "__main__":
    try:
        main()
    except BaseException as error:  # yt-dlp exposes several non-standard extractor errors
        emit({"type": "error", "message": safe_error(error)})
        raise SystemExit(1)
