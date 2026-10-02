import { Channel, invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { open, save } from "@tauri-apps/plugin-dialog";
import {
  isPermissionGranted,
  requestPermission,
  sendNotification,
} from "@tauri-apps/plugin-notification";
import { relaunch } from "@tauri-apps/plugin-process";
import { check, type DownloadEvent, type Update } from "@tauri-apps/plugin-updater";
import { FormEvent, ReactNode, useEffect, useMemo, useRef, useState } from "react";
import "./App.css";
import { MessageKey, translate } from "./i18n";
import quiverLogo from "../src-tauri/icons/icon.png";

type LinkInspectionResponse = {
  effectiveUrl: string;
  totalBytes: string | null;
  supportsRanges: boolean;
  hasValidator: boolean;
  suggestedFilename: string;
  contentType: string | null;
};

type LinkInspection = LinkInspectionResponse & {
  sourceUrl: string;
};

type MediaFormat = {
  formatId: string;
  label: string;
  height: number | null;
  extension: string;
  audioOnly: boolean;
  hasAudio: boolean;
  approxBytes: string | null;
};

type MediaInspection = {
  sourceUrl: string;
  title: string;
  extractor: string;
  thumbnail: string | null;
  durationSeconds: number | null;
  formats: MediaFormat[];
};

type EngineStatus = "probing" | "retrying" | "downloading" | "verifying" | "completed";
type DownloadStatus =
  | EngineStatus
  | "starting"
  | "queued"
  | "scheduled"
  | "paused"
  | "extracting"
  | "cancelling"
  | "cancelled"
  | "failed";

type DownloadProgress = {
  status: EngineStatus;
  downloadedBytes: string;
  totalBytes: string | null;
};

type DownloadSummary = {
  bytesWritten: string;
  sha256: string;
  resumed: boolean;
};

type DownloadItem = {
  id: string;
  name: string;
  url: string;
  destination: string;
  status: DownloadStatus;
  downloadedBytes: bigint;
  totalBytes: bigint | null;
  sha256?: string;
  resumed?: boolean;
  error?: string;
  recoverable?: boolean;
  queuedAtMs: string | null;
  scheduledForMs: string | null;
  queueSequence: string | null;
  completedAtMs: string | null;
  kind: "direct" | "media" | "torrent";
  mediaQuality?: string;
  torrentSelection?: number[];
};

type CategoryRule = {
  name: string;
  folder: string;
  extensions: string[];
  mimePrefixes: string[];
};

type AppSettings = {
  theme: "system" | "light" | "dark";
  accentColor: string | null;
  language: "en" | "ar";
  notifications: boolean;
  retryAttempts: number;
  retryInitialDelayMs: number;
  retryMaxDelayMs: number;
  maxSegments: number;
  maxConnectionsPerHost: number;
  perDownloadSpeedLimitBps: number | null;
  globalSpeedLimitBps: number | null;
  queueMode: "parallel" | "sequential";
  historyRetentionDays: 7 | 30 | 90 | null;
  proxyMode: "disabled" | "system" | "custom";
  proxyUrl: string;
  proxyUsername: string;
  proxyBypass: string;
  clipboardMonitoring: boolean;
  smartRouting: boolean;
  defaultDownloadPath: string;
  categories: CategoryRule[];
  mediaPythonPath: string;
};

type ProxyDraft = Pick<
  AppSettings,
  "proxyMode" | "proxyUrl" | "proxyUsername" | "proxyBypass"
>;

type StoredDownload = Omit<DownloadItem, "downloadedBytes" | "totalBytes" | "recoverable"> & {
  downloadedBytes: string;
  totalBytes: string | null;
};

type AppSnapshot = {
  schemaVersion: number;
  settings: AppSettings;
  downloads: StoredDownload[];
};

type BrowserRequest = {
  version: number;
  id: string;
  url: string;
  suggestedFilename: string | null;
  automatic: boolean;
};

type BrowserBridgeInfo = {
  connected: boolean;
  message: string;
};

type ClipboardCandidate = {
  url: string;
  kind: DownloadItem["kind"];
};

type MediaProgress = {
  status: "downloading" | "verifying" | "extracting";
  downloadedBytes: string;
  totalBytes: string | null;
};

type MediaSummary = {
  destination: string;
  bytesWritten: string;
};

type TorrentInspection = {
  sourceUrl: string;
  name: string;
  sourceType: "magnet" | "torrentFile";
  networkOrigins: string[];
  files?: { path: string; bytes: string }[];
  totalBytes?: string | null;
};

type TorrentProgress = {
  status: "probing" | "downloading" | "paused" | "failed";
  downloadedBytes: string;
  totalBytes: string | null;
  name: string | null;
};

type TorrentSummary = {
  destination: string;
  bytesWritten: string;
  name: string;
};

type SourceMode = "auto" | "media" | "torrent";

type Filter = "all" | "active" | "completed" | "failed";
type HistorySort = "newest" | "oldest" | "name" | "size";

const SETTINGS_TABS = [
  { id: "general", en: "General", ar: "عام", words: "theme appearance accent color light dark system language notifications history retention مظهر لغة إشعارات سجل" },
  { id: "downloads", en: "Downloads", ar: "التنزيلات", words: "retry attempts connections segments server speed bandwidth limits queue parallel sequential محاولات سرعة اتصالات طابور" },
  { id: "folders", en: "Folders", ar: "المجلدات", words: "default download folder directory path smart routing categories extensions mime حفظ مجلد مسار تصنيف" },
  { id: "network", en: "Network", ar: "الشبكة", words: "proxy direct system custom url bypass username password network وكيل شبكة كلمة مرور" },
  { id: "integration", en: "Browser", ar: "المتصفح", words: "firefox browser extension integration capture clipboard copied links متصفح فايرفوكس روابط حافظة" },
  { id: "media", en: "Media", ar: "الوسائط", words: "python executable yt-dlp ffmpeg audio video engine وسائط فيديو صوت" },
  { id: "updates", en: "Updates", ar: "التحديثات", words: "updates version install release تحديث إصدار" },
] as const;
type SettingsTab = typeof SETTINGS_TABS[number]["id"];

function SettingsSection({ title, visible, children }: { title: string; visible: boolean; children: ReactNode }) {
  if (!visible) return null;
  return <section className="settings-section" aria-label={title}><h3>{title}</h3>{children}</section>;
}

const APP_STATE_SCHEMA_VERSION = 5;
const DAY_MS = 24 * 60 * 60 * 1000;
const UPDATE_CHECK_INTERVAL_MS = 6 * 60 * 60 * 1000;
const UPDATER_ENABLED = import.meta.env.VITE_QUIVERDL_UPDATER === "true";

const ACTIVE_STATUSES = new Set<DownloadStatus>([
  "starting",
  "queued",
  "scheduled",
  "probing",
  "retrying",
  "downloading",
  "paused",
  "extracting",
  "verifying",
  "cancelling",
]);

const STATUS_LABELS: Record<DownloadStatus, string> = {
  starting: "Starting",
  queued: "Queued",
  scheduled: "Scheduled",
  probing: "Inspecting server",
  retrying: "Retrying",
  downloading: "Downloading",
  paused: "Paused",
  extracting: "Extracting audio",
  verifying: "Verifying",
  cancelling: "Cancelling",
  completed: "Completed",
  cancelled: "Cancelled",
  failed: "Failed",
};

const DEFAULT_SETTINGS: AppSettings = {
  theme: "system",
  accentColor: null,
  language: "en",
  notifications: true,
  retryAttempts: 3,
  retryInitialDelayMs: 750,
  retryMaxDelayMs: 15_000,
  maxSegments: 4,
  maxConnectionsPerHost: 8,
  perDownloadSpeedLimitBps: null,
  globalSpeedLimitBps: null,
  queueMode: "parallel",
  historyRetentionDays: null,
  proxyMode: "disabled",
  proxyUrl: "",
  proxyUsername: "",
  proxyBypass: "",
  clipboardMonitoring: false,
  smartRouting: true,
  defaultDownloadPath: "",
  categories: [
    { name: "Compressed", folder: "Compressed", extensions: [".zip", ".rar", ".7z", ".tar", ".tar.gz", ".gz", ".bz2", ".xz"], mimePrefixes: ["application/zip", "application/x-rar", "application/x-7z", "application/gzip"] },
    { name: "Video", folder: "Video", extensions: [".mp4", ".mkv", ".avi", ".mov", ".webm", ".m4v"], mimePrefixes: ["video/"] },
    { name: "Audio", folder: "Audio", extensions: [".mp3", ".wav", ".flac", ".m4a", ".aac", ".ogg", ".opus"], mimePrefixes: ["audio/"] },
    { name: "Documents", folder: "Documents", extensions: [".pdf", ".doc", ".docx", ".xls", ".xlsx", ".ppt", ".pptx", ".txt", ".epub"], mimePrefixes: ["application/pdf", "text/"] },
    { name: "Software", folder: "Software", extensions: [".exe", ".msi", ".msix", ".dmg", ".pkg", ".deb", ".rpm", ".appimage"], mimePrefixes: ["application/x-msdownload", "application/vnd.microsoft.portable-executable"] },
    { name: "Torrents", folder: "Torrents", extensions: [".torrent"], mimePrefixes: ["application/x-bittorrent"] },
  ],
  mediaPythonPath: "",
};

const THEME_ACCENTS = [
  { name: "Ocean", color: "#62A7FF" },
  { name: "Cyan", color: "#22C1D6" },
  { name: "Violet", color: "#9B87F5" },
  { name: "Emerald", color: "#35C98B" },
  { name: "Amber", color: "#F4B942" },
  { name: "Rose", color: "#F06C91" },
] as const;

function rgbChannels(color: string) {
  return [
    Number.parseInt(color.slice(1, 3), 16),
    Number.parseInt(color.slice(3, 5), 16),
    Number.parseInt(color.slice(5, 7), 16),
  ] as const;
}

function relativeLuminance(color: string) {
  const linearChannel = (value: number) => {
    const channel = value / 255;
    return channel <= 0.04045 ? channel / 12.92 : ((channel + 0.055) / 1.055) ** 2.4;
  };
  const [red, green, blue] = rgbChannels(color);
  return 0.2126 * linearChannel(red) + 0.7152 * linearChannel(green) + 0.0722 * linearChannel(blue);
}

function contrastRatio(first: string, second: string) {
  const brighter = Math.max(relativeLuminance(first), relativeLuminance(second));
  const darker = Math.min(relativeLuminance(first), relativeLuminance(second));
  return (brighter + 0.05) / (darker + 0.05);
}

function accentInk(color: string) {
  const luminance = relativeLuminance(color);
  const blackContrast = (luminance + 0.05) / 0.05;
  const whiteContrast = 1.05 / (luminance + 0.05);
  return blackContrast >= whiteContrast ? "#000000" : "#ffffff";
}

function mixHex(color: string, target: string, amount: number) {
  const sourceChannels = rgbChannels(color);
  const targetChannels = rgbChannels(target);
  return `#${sourceChannels.map((channel, index) =>
    Math.round(channel + (targetChannels[index] - channel) * amount)
      .toString(16)
      .padStart(2, "0"),
  ).join("")}`;
}

function accessibleAccentForeground(color: string, lightTheme: boolean) {
  const surfaces = lightTheme ? ["#edf3fa", "#f7faff"] : ["#07101f", "#0c1b30"];
  const target = lightTheme ? "#000000" : "#ffffff";
  for (let step = 0; step <= 20; step += 1) {
    const candidate = mixHex(color, target, step / 20);
    if (surfaces.every((surface) => contrastRatio(candidate, surface) >= 4.5)) return candidate;
  }
  return target;
}

function matchingCategory(
  filename: string,
  contentType: string | null,
  categories: CategoryRule[],
) {
  const lowerName = filename.toLowerCase();
  const lowerMime = contentType?.split(";", 1)[0].trim().toLowerCase() ?? "";
  return categories.find((category) =>
    category.extensions.some((extension) => lowerName.endsWith(extension.toLowerCase()))
      || category.mimePrefixes.some((prefix) => lowerMime.startsWith(prefix.toLowerCase())),
  ) ?? null;
}

function isLikelyMediaUrl(value: string) {
  try {
    const hostname = new URL(value).hostname.toLowerCase().replace(/^www\./, "");
    return [
      "youtube.com",
      "youtu.be",
      "vimeo.com",
      "dailymotion.com",
      "twitch.tv",
      "tiktok.com",
      "instagram.com",
      "facebook.com",
      "soundcloud.com",
      "x.com",
      "twitter.com",
    ].some((domain) => hostname === domain || hostname.endsWith(`.${domain}`));
  } catch {
    return false;
  }
}

function proxyDraftFromSettings(settings: AppSettings): ProxyDraft {
  return {
    proxyMode: settings.proxyMode,
    proxyUrl: settings.proxyUrl,
    proxyUsername: settings.proxyUsername,
    proxyBypass: settings.proxyBypass,
  };
}

const FILTER_LABELS: Record<Exclude<Filter, "all">, string> = {
  active: "Active downloads",
  completed: "Completed downloads",
  failed: "Needs attention",
};

function formatBytes(bytes: bigint | null) {
  if (bytes === null) return "Unknown";
  const units = ["B", "KB", "MB", "GB", "TB"];
  let divisor = 1n;
  let unit = 0;
  while (bytes >= divisor * 1024n && unit < units.length - 1) {
    divisor *= 1024n;
    unit += 1;
  }
  if (unit === 0) return `${bytes} ${units[unit]}`;

  const tenths = (bytes * 10n + divisor / 2n) / divisor;
  const whole = tenths / 10n;
  const fraction = tenths % 10n;
  return `${whole}${fraction === 0n ? "" : `.${fraction}`} ${units[unit]}`;
}

function filenameFromUrl(value: string) {
  try {
    const segments = new URL(value).pathname.split("/").filter(Boolean);
    const segment = segments[segments.length - 1];
    const decoded = segment ? decodeURIComponent(segment) : "download.bin";
    const safe = decoded
      .replace(/[<>:"/\\|?*\u0000-\u001f]/g, "_")
      .replace(/[.\s]+$/g, "")
      .slice(0, 180);
    return safe || "download.bin";
  } catch {
    return "download.bin";
  }
}

function destinationName(path: string) {
  const segments = path.split(/[\\/]/).filter(Boolean);
  return segments[segments.length - 1] ?? "download";
}

function createTaskId() {
  return globalThis.crypto?.randomUUID?.() ??
    `download-${Date.now()}-${Math.random().toString(16).slice(2)}`;
}

function queueStatus(item: Pick<DownloadItem, "scheduledForMs">, settings: AppSettings) {
  if (item.scheduledForMs !== null && Number(item.scheduledForMs) > Date.now()) {
    return "scheduled" as const;
  }
  return settings.queueMode === "sequential" ? "queued" as const : "starting" as const;
}

function compareQueueOrder(left: DownloadItem, right: DownloadItem) {
  if (left.queueSequence !== null && right.queueSequence !== null) {
    const leftSequence = BigInt(left.queueSequence);
    const rightSequence = BigInt(right.queueSequence);
    if (leftSequence < rightSequence) return -1;
    if (leftSequence > rightSequence) return 1;
  }
  return left.id.localeCompare(right.id);
}

function completedTimestamp(item: DownloadItem) {
  return item.completedAtMs === null ? null : Number(item.completedAtMs);
}

function compareHistory(left: DownloadItem, right: DownloadItem, sort: HistorySort) {
  if (sort === "name") return left.name.localeCompare(right.name, undefined, { sensitivity: "base" });
  if (sort === "size") {
    const sizeOrder = right.downloadedBytes < left.downloadedBytes
      ? -1
      : right.downloadedBytes > left.downloadedBytes ? 1 : 0;
    return sizeOrder || left.name.localeCompare(right.name, undefined, { sensitivity: "base" });
  }

  const leftTimestamp = completedTimestamp(left);
  const rightTimestamp = completedTimestamp(right);
  if (leftTimestamp === null && rightTimestamp === null) return left.name.localeCompare(right.name);
  if (leftTimestamp === null) return 1;
  if (rightTimestamp === null) return -1;
  return sort === "oldest"
    ? leftTimestamp - rightTimestamp
    : rightTimestamp - leftTimestamp;
}

function pruneCompletedHistory(
  items: DownloadItem[],
  retentionDays: AppSettings["historyRetentionDays"],
  now = Date.now(),
) {
  if (retentionDays === null) return items;
  const cutoff = now - retentionDays * DAY_MS;
  return items.filter((item) => {
    if (item.status !== "completed" || item.completedAtMs === null) return true;
    return Number(item.completedAtMs) >= cutoff;
  });
}

function formatHistoryTime(timestamp: string, language: AppSettings["language"]) {
  return new Intl.DateTimeFormat(language === "ar" ? "ar" : "en", {
    dateStyle: "medium",
    timeStyle: "short",
  }).format(new Date(Number(timestamp)));
}

const MAX_QUEUE_SEQUENCE = (1n << 64n) - 1n;

function formatScheduledTime(timestamp: string) {
  return new Intl.DateTimeFormat(undefined, {
    dateStyle: "medium",
    timeStyle: "short",
  }).format(new Date(Number(timestamp)));
}

function App() {
  const [url, setUrl] = useState("");
  const [inspection, setInspection] = useState<LinkInspection | null>(null);
  const [mediaInspection, setMediaInspection] = useState<MediaInspection | null>(null);
  const [torrentInspection, setTorrentInspection] = useState<TorrentInspection | null>(null);
  const [torrentPrivacyConfirmed, setTorrentPrivacyConfirmed] = useState(false);
  const [torrentDestination, setTorrentDestination] = useState("");
  const [selectedTorrentFiles, setSelectedTorrentFiles] = useState<Set<number>>(new Set());
  const [torrentFileSearch, setTorrentFileSearch] = useState("");
  const [torrentMetadataBusy, setTorrentMetadataBusy] = useState(false);
  const [torrentMenu, setTorrentMenu] = useState<{ x: number; y: number; index?: number } | null>(null);
  const [mediaQuality, setMediaQuality] = useState("best");
  const [sourceMode, setSourceMode] = useState<SourceMode>("auto");
  const [downloads, setDownloads] = useState<DownloadItem[]>([]);
  const [filter, setFilter] = useState<Filter>("all");
  const [historyQuery, setHistoryQuery] = useState("");
  const [historySort, setHistorySort] = useState<HistorySort>("newest");
  const [error, setError] = useState("");
  const [updateError, setUpdateError] = useState("");
  const [inspecting, setInspecting] = useState(false);
  const [choosingDestination, setChoosingDestination] = useState(false);
  const [scheduledStart, setScheduledStart] = useState("");
  const [settings, setSettings] = useState<AppSettings>(DEFAULT_SETTINGS);
  const latestSettings = useRef(settings);
  latestSettings.current = settings;
  const [settingsTab, setSettingsTab] = useState<SettingsTab>("general");
  const [settingsSearch, setSettingsSearch] = useState("");
  const settingsMatches = (id: SettingsTab) => {
    const query = settingsSearch.trim().toLocaleLowerCase();
    if (!query) return id === settingsTab;
    const tab = SETTINGS_TABS.find((entry) => entry.id === id)!;
    const index = `${tab.en} ${tab.ar} ${tab.words}`.toLocaleLowerCase();
    return query.split(/\s+/).every((word) => index.includes(word));
  };
  const [browserRequests, setBrowserRequests] = useState<BrowserRequest[]>([]);
  const browserHandled = useRef(new Set<string>());
  const browserProcessing = useRef(false);
  const openedSourceProcessing = useRef(false);
  const [openedSources, setOpenedSources] = useState<string[]>([]);
  const [reviewingBrowserRequest, setReviewingBrowserRequest] =
    useState<BrowserRequest | null>(null);
  const [bridgeInfo, setBridgeInfo] = useState<BrowserBridgeInfo | null>(null);
  const [proxyPassword, setProxyPassword] = useState("");
  const [proxyDraft, setProxyDraft] = useState<ProxyDraft>(
    proxyDraftFromSettings(DEFAULT_SETTINGS),
  );
  const [proxyCredentialsPresent, setProxyCredentialsPresent] = useState(false);
  const [proxyCredentialBusy, setProxyCredentialBusy] = useState(false);
  const stateLoaded = useRef(false);
  const recoveryQueue = useRef<DownloadItem[]>([]);
  const registeredDownloads = useRef(new Set<string>());
  const pendingCancellations = useRef(new Set<string>());
  const nextQueueSequence = useRef(0n);
  const recoveryGate = useRef<{ promise: Promise<void>; release: () => void } | null>(null);
  if (recoveryGate.current === null) {
    let release: () => void = () => undefined;
    const promise = new Promise<void>((resolve) => {
      release = resolve;
    });
    recoveryGate.current = { promise, release };
  }
  const [stateReady, setStateReady] = useState(false);
  const saveTimer = useRef<number | null>(null);
  const saveInFlight = useRef(false);
  const saveAgain = useRef(false);
  const latestSnapshot = useRef<AppSnapshot | null>(null);
  const quitInProgress = useRef(false);
  const updaterOperation = useRef<"check" | "action" | null>(null);
  const availableUpdate = useRef<Update | null>(null);
  const [availableUpdateVersion, setAvailableUpdateVersion] = useState<string | null>(null);
  const [updateChecking, setUpdateChecking] = useState(false);
  const [updateBusy, setUpdateBusy] = useState(false);
  const [updateDownloaded, setUpdateDownloaded] = useState(false);
  const [updateProgress, setUpdateProgress] = useState<number | null>(null);
  const [updateStatus, setUpdateStatus] = useState("");
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [clipboardCandidate, setClipboardCandidate] = useState<ClipboardCandidate | null>(null);
  const urlInputRef = useRef<HTMLInputElement>(null);
  const settingsModalRef = useRef<HTMLElement>(null);
  const settingsOpenerRef = useRef<HTMLElement | null>(null);
  const t = (key: MessageKey) => translate(settings.language, key);

  useEffect(() => {
    let active = true;
    void invoke<AppSnapshot>("load_app_state")
      .then((snapshot) => {
        if (!active) return;
        setSettings(snapshot.settings);
        setProxyDraft(proxyDraftFromSettings(snapshot.settings));
        const restored = pruneCompletedHistory(snapshot.downloads.map((item) => ({
          ...item,
          kind: item.kind ?? "direct",
          downloadedBytes: BigInt(item.downloadedBytes),
          totalBytes: item.totalBytes === null ? null : BigInt(item.totalBytes),
          recoverable: item.status === "paused",
        })), snapshot.settings.historyRetentionDays);
        nextQueueSequence.current = restored.reduce((next, item) => {
          if (item.queueSequence === null) return next;
          const sequence = BigInt(item.queueSequence);
          return sequence >= next ? sequence + 1n : next;
        }, 0n);
        recoveryQueue.current = restored
          .filter((item) => item.status === "queued" || item.status === "scheduled")
          .sort(compareQueueOrder);
        setDownloads(restored);
        stateLoaded.current = true;
        setStateReady(true);
      })
      .catch((cause) => {
        if (active) setError(String(cause));
        stateLoaded.current = true;
        setStateReady(true);
      });
    return () => {
      active = false;
    };
  }, []);

  useEffect(() => {
    if (!stateReady) return;
    const pending = recoveryQueue.current;
    recoveryQueue.current = [];
    void (async () => {
      const registered: DownloadItem[] = [];
      for (const item of pending) {
        if (item.kind === "torrent") {
          updateDownload(item.id, {
            status: "failed",
            error: "Review and confirm the BitTorrent privacy disclosure again after restarting QuiverDL.",
          });
          continue;
        }
        if (await registerDownload(item, settings)) registered.push(item);
      }
      recoveryGate.current?.release();
      for (const item of registered) {
        if (item.kind === "media") void executeMediaDownload(item, settings);
        else void executeDownload(item, settings);
      }
    })();
  }, [stateReady]);

  useEffect(() => {
    let active = true;
    const refresh = () => {
      void invoke<string[]>("take_open_requests").then((sources) => {
        if (active && sources.length) setOpenedSources((current) => [...current, ...sources]);
      });
      void invoke<BrowserRequest[]>("list_browser_requests")
        .then((requests) => {
          if (active) setBrowserRequests(requests);
        })
        .catch((cause) => {
          if (active) setError(`Could not read browser download requests: ${String(cause)}`);
        });
    };
    refresh();
    const interval = window.setInterval(refresh, 3_000);
    return () => {
      active = false;
      window.clearInterval(interval);
    };
  }, []);

  useEffect(() => {
    if (!stateReady || browserProcessing.current) return;
    const request = browserRequests.find((entry) => entry.automatic && !browserHandled.current.has(entry.id));
    if (!request) return;
    // A torrent awaiting the user's download choice must not be replaced.
    if (torrentInspection) return;
    browserProcessing.current = true;
    browserHandled.current.add(request.id);
    void (async () => {
      try {
        if (latestSnapshot.current?.downloads.some((entry) => entry.id === request.id)) {
          await invoke("acknowledge_browser_request", { id: request.id });
          return;
        }
        const torrent = request.url.toLowerCase().startsWith("magnet:") || /\.torrent(?:[?#]|$)/i.test(request.suggestedFilename || request.url);
        if (torrent) {
          const result = await invoke<TorrentInspection>("inspect_torrent_source", { source: request.url, settings: latestSettings.current });
          setTorrentInspection(result);
          setReviewingBrowserRequest(request);
          setSourceMode("torrent");
          return;
        }
        const result = await invoke<LinkInspectionResponse>("inspect_url", { url: request.url, settings: latestSettings.current });
        const filename = request.suggestedFilename || result.suggestedFilename || filenameFromUrl(result.effectiveUrl);
        if (result.contentType?.split(";", 1)[0].trim() === "application/x-bittorrent" || /\.torrent$/i.test(result.suggestedFilename || filename)) {
          const preview = await invoke<TorrentInspection>("inspect_torrent_source", { source: request.url, settings: latestSettings.current });
          setTorrentInspection(preview);
          setReviewingBrowserRequest(request);
          setSourceMode("torrent");
          return;
        }
        const current = latestSettings.current;
        const category = matchingCategory(filename, result.contentType, current.categories);
        const destination = await invoke<string>("resolve_browser_destination", {
          defaultPath: current.defaultDownloadPath,
          categoryFolder: current.smartRouting ? category?.folder || "" : "",
          filename,
        });
        await runDownload(request.url, destination, undefined, request.id);
      } catch {
        setError("Could not start a browser download. Use its Review button to retry.");
      } finally { browserProcessing.current = false; }
    })();
  }, [stateReady, browserRequests, torrentInspection]);

  useEffect(() => {
    if (!stateReady || torrentInspection || !openedSources.length || openedSourceProcessing.current) return;
    openedSourceProcessing.current = true;
    const source = openedSources[0];
    void invoke<TorrentInspection>("inspect_torrent_source", { source, settings: latestSettings.current }).then((result) => {
      setTorrentInspection(result);
      setReviewingBrowserRequest(null);
      setSourceMode("torrent");
    }).catch((cause) => setError(String(cause))).finally(() => {
      openedSourceProcessing.current = false;
      setOpenedSources((current) => current.slice(1));
    });
  }, [stateReady, openedSources, torrentInspection]);

  useEffect(() => {
    latestSnapshot.current = {
      schemaVersion: APP_STATE_SCHEMA_VERSION,
      settings,
      downloads: downloads.map(({ recoverable: _recoverable, ...item }) => ({
        ...item,
        downloadedBytes: item.downloadedBytes.toString(),
        totalBytes: item.totalBytes?.toString() ?? null,
      })),
    };
    if (
      !stateLoaded.current ||
      quitInProgress.current ||
      saveTimer.current !== null
    )
      return;
    saveTimer.current = window.setTimeout(() => {
      saveTimer.current = null;
      if (saveInFlight.current) {
        saveAgain.current = true;
        return;
      }
      saveInFlight.current = true;
      void (async () => {
        try {
          do {
            saveAgain.current = false;
            if (latestSnapshot.current) {
              await invoke("save_app_state", { snapshot: latestSnapshot.current });
            }
          } while (saveAgain.current);
        } catch (cause) {
          setError(String(cause));
        } finally {
          saveInFlight.current = false;
        }
      })();
    }, 500);
  }, [downloads, settings]);

  useEffect(() => {
    if (!stateReady || settings.historyRetentionDays === null) return;
    const prune = () => setDownloads((current) => {
      const retained = pruneCompletedHistory(current, settings.historyRetentionDays);
      return retained.length === current.length ? current : retained;
    });
    prune();
    const timer = window.setInterval(prune, 60 * 60 * 1000);
    return () => window.clearInterval(timer);
  }, [stateReady, settings.historyRetentionDays]);

  useEffect(() => {
    if (!stateReady || !UPDATER_ENABLED) return;
    void checkForUpdates(false);
    const timer = window.setInterval(() => void checkForUpdates(false), UPDATE_CHECK_INTERVAL_MS);
    return () => window.clearInterval(timer);
  }, [stateReady]);

  useEffect(
    () => () => {
      void availableUpdate.current?.close();
    },
    [],
  );

  useEffect(
    () => () => {
      if (saveTimer.current !== null) window.clearTimeout(saveTimer.current);
    },
    [],
  );

  useEffect(() => {
    let active = true;
    let stopListening: (() => void) | undefined;
    void listen("quit-requested", async () => {
      if (!active || quitInProgress.current) return;
      quitInProgress.current = true;
      if (saveTimer.current !== null) {
        window.clearTimeout(saveTimer.current);
        saveTimer.current = null;
      }
      try {
        while (saveInFlight.current) {
          await new Promise((resolve) => window.setTimeout(resolve, 25));
        }
        if (latestSnapshot.current) {
          await invoke("save_app_state", { snapshot: latestSnapshot.current });
        }
        await invoke("quit_app");
      } catch (cause) {
        quitInProgress.current = false;
        setError(String(cause));
      }
    }).then((unlisten) => {
      if (active) stopListening = unlisten;
      else unlisten();
    });
    return () => {
      active = false;
      stopListening?.();
    };
  }, []);

  useEffect(() => {
    document.documentElement.dataset.theme = settings.theme;
    document.documentElement.lang = settings.language;
    document.documentElement.dir = settings.language === "ar" ? "rtl" : "ltr";
    const preferredLightTheme = window.matchMedia("(prefers-color-scheme: light)");
    const applyAccent = () => {
      if (!settings.accentColor) return;
      const ink = accentInk(settings.accentColor);
      const hover = mixHex(settings.accentColor, ink === "#000000" ? "#ffffff" : "#000000", 0.12);
      const lightTheme = settings.theme === "light" || (settings.theme === "system" && preferredLightTheme.matches);
      document.documentElement.style.setProperty("--user-accent", settings.accentColor);
      document.documentElement.style.setProperty("--user-accent-ink", ink);
      document.documentElement.style.setProperty("--user-accent-hover", hover);
      document.documentElement.style.setProperty("--user-accent-hover-ink", accentInk(hover));
      document.documentElement.style.setProperty("--user-accent-fg", accessibleAccentForeground(settings.accentColor, lightTheme));
    };
    if (settings.accentColor) {
      applyAccent();
      preferredLightTheme.addEventListener("change", applyAccent);
    } else {
      document.documentElement.style.removeProperty("--user-accent");
      document.documentElement.style.removeProperty("--user-accent-ink");
      document.documentElement.style.removeProperty("--user-accent-hover");
      document.documentElement.style.removeProperty("--user-accent-hover-ink");
      document.documentElement.style.removeProperty("--user-accent-fg");
    }
    return () => preferredLightTheme.removeEventListener("change", applyAccent);
  }, [settings.accentColor, settings.language, settings.theme]);

  useEffect(() => {
    if (!stateReady) return;
    void invoke("set_clipboard_monitor_enabled", {
      enabled: settings.clipboardMonitoring,
    }).catch((cause) => setError(`Could not update clipboard monitoring: ${String(cause)}`));
  }, [settings.clipboardMonitoring, stateReady]);

  useEffect(() => {
    let active = true;
    let stopListening: (() => void) | undefined;
    void listen<ClipboardCandidate>("clipboard-download-candidate", (event) => {
      if (active) setClipboardCandidate(event.payload);
    }).then((unlisten) => {
      if (active) stopListening = unlisten;
      else unlisten();
    });
    return () => {
      active = false;
      stopListening?.();
    };
  }, []);

  useEffect(() => {
    setTorrentPrivacyConfirmed(false);
    setSelectedTorrentFiles(new Set(torrentInspection?.files?.map((_, index) => index) || []));
    setTorrentFileSearch("");
    setTorrentMenu(null);
    if (torrentInspection && !torrentDestination) {
      if (settings.defaultDownloadPath) setTorrentDestination(settings.defaultDownloadPath);
      else void invoke<string>("default_download_directory").then(setTorrentDestination).catch(() => {});
    }
  }, [torrentInspection?.sourceUrl]);

  useEffect(() => {
    if (!torrentMenu) return;
    const close = (event: MouseEvent) => { if (!(event.target as HTMLElement).closest(".torrent-context-menu")) setTorrentMenu(null); };
    const escape = (event: KeyboardEvent) => { if (event.key === "Escape") setTorrentMenu(null); };
    window.addEventListener("mousedown", close);
    window.addEventListener("keydown", escape);
    return () => { window.removeEventListener("mousedown", close); window.removeEventListener("keydown", escape); };
  }, [torrentMenu]);

  useEffect(() => {
    if (!settingsOpen) return;
    const modal = settingsModalRef.current;
    const opener = settingsOpenerRef.current;
    const focusableElements = () => Array.from(
      modal?.querySelectorAll<HTMLElement>(
        'button:not([disabled]), input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [href], [tabindex]:not([tabindex="-1"])',
      ) ?? [],
    ).filter((element) => element.getClientRects().length > 0);
    const containFocus = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        setSettingsOpen(false);
        return;
      }
      if (event.key !== "Tab") return;
      const focusable = focusableElements();
      if (focusable.length === 0) {
        event.preventDefault();
        modal?.focus();
        return;
      }
      const first = focusable[0];
      const last = focusable[focusable.length - 1];
      const active = document.activeElement;
      if (event.shiftKey && (active === first || !modal?.contains(active))) {
        event.preventDefault();
        last.focus();
      } else if (!event.shiftKey && active === last) {
        event.preventDefault();
        first.focus();
      }
    };
    window.addEventListener("keydown", containFocus);
    return () => {
      window.removeEventListener("keydown", containFocus);
      opener?.focus();
    };
  }, [settingsOpen]);

  function openSettings(opener: HTMLElement) {
    settingsOpenerRef.current = opener;
    setSettingsOpen(true);
  }

  function closeSettings() {
    setSettingsOpen(false);
  }

  function activateSourceMode(mode: SourceMode) {
    setSourceMode(mode);
    setFilter("all");
    setError("");
    setInspection(null);
    setMediaInspection(null);
    setTorrentInspection(null);
    setReviewingBrowserRequest(null);
    window.requestAnimationFrame(() => urlInputRef.current?.focus());
  }

  useEffect(() => {
    void invoke("set_global_speed_limit", {
      bytesPerSecond: settings.globalSpeedLimitBps,
    }).catch((cause) => setError(String(cause)));
  }, [settings.globalSpeedLimitBps]);

  useEffect(() => {
    let active = true;
    if (proxyDraft.proxyMode !== "custom" || !proxyDraft.proxyUsername) {
      setProxyCredentialsPresent(false);
      return () => {
        active = false;
      };
    }
    const timer = window.setTimeout(() => {
      void invoke<boolean>("has_proxy_credentials", {
        endpoint: proxyDraft.proxyUrl,
        username: proxyDraft.proxyUsername,
      })
        .then((present) => {
          if (active) setProxyCredentialsPresent(present);
        })
        .catch((cause) => {
          if (active) {
            setProxyCredentialsPresent(false);
            setError(String(cause));
          }
        });
    }, 300);
    return () => {
      active = false;
      window.clearTimeout(timer);
    };
  }, [proxyDraft.proxyMode, proxyDraft.proxyUrl, proxyDraft.proxyUsername]);

  const counts = useMemo(
    () => ({
      active: downloads.filter((item) => ACTIVE_STATUSES.has(item.status)).length,
      completed: downloads.filter((item) => item.status === "completed").length,
      failed: downloads.filter(
        (item) => item.status === "failed" || item.status === "cancelled",
      ).length,
    }),
    [downloads],
  );

  const visibleDownloads = useMemo(() => {
    const filtered = downloads.filter((item) => {
        if (filter === "active") return ACTIVE_STATUSES.has(item.status);
        if (filter === "completed") return item.status === "completed";
        if (filter === "failed") {
          return item.status === "failed" || item.status === "cancelled";
        }
        return true;
      });
    if (filter !== "completed") return filtered;
    const query = historyQuery.trim().toLocaleLowerCase();
    return filtered
      .filter((item) => !query || [item.name, item.destination, item.url]
        .some((value) => value.toLocaleLowerCase().includes(query)))
      .sort((left, right) => compareHistory(left, right, historySort));
  }, [downloads, filter, historyQuery, historySort]);

  function updateDownload(
    id: string,
    update: Partial<DownloadItem> | ((item: DownloadItem) => Partial<DownloadItem>),
  ) {
    setDownloads((current) =>
      current.map((item) => {
        if (item.id !== id) return item;
        return { ...item, ...(typeof update === "function" ? update(item) : update) };
      }),
    );
  }

  async function persistSnapshotNow(snapshot: AppSnapshot) {
    latestSnapshot.current = snapshot;
    if (saveTimer.current !== null) {
      window.clearTimeout(saveTimer.current);
      saveTimer.current = null;
    }
    if (saveInFlight.current) {
      saveAgain.current = true;
      while (saveInFlight.current) {
        await new Promise((resolve) => window.setTimeout(resolve, 25));
      }
    }

    saveInFlight.current = true;
    try {
      do {
        saveAgain.current = false;
        if (latestSnapshot.current) {
          await invoke("save_app_state", { snapshot: latestSnapshot.current });
        }
      } while (saveAgain.current);
    } finally {
      saveInFlight.current = false;
    }
  }

  async function inspectLink(event: FormEvent) {
    event.preventDefault();
    const submittedUrl = url.trim();
    if (!submittedUrl) return;
    setInspecting(true);
    setError("");
    setInspection(null);
    setMediaInspection(null);
    setTorrentInspection(null);
    try {
      if (
        sourceMode === "torrent"
        || submittedUrl.toLowerCase().startsWith("magnet:")
        || submittedUrl.toLowerCase().split(/[?#]/, 1)[0].endsWith(".torrent")
      ) {
        const result = await invoke<TorrentInspection>("inspect_torrent_source", {
          source: submittedUrl,
          settings,
        });
        setTorrentInspection(result);
        return;
      }
      let useMedia = sourceMode === "media" || isLikelyMediaUrl(submittedUrl);
      if (sourceMode === "auto") {
        if (!useMedia) {
          try {
            useMedia = await invoke<boolean>("detect_media_url", {
              url: submittedUrl,
              settings,
            });
          } catch {
            useMedia = false;
          }
        }
      }
      if (useMedia) {
        const metadata = await invoke<Omit<MediaInspection, "sourceUrl">>("inspect_media_url", {
          url: submittedUrl,
          settings,
        });
        setMediaInspection({ ...metadata, sourceUrl: submittedUrl });
        setMediaQuality("best");
        return;
      }
      const result = await invoke<LinkInspectionResponse>("inspect_url", {
        url: submittedUrl,
        settings,
      });
      setInspection({ ...result, sourceUrl: submittedUrl });
    } catch (cause) {
      setError(String(cause));
    } finally {
      setInspecting(false);
    }
  }

  async function chooseDestination() {
    if (!inspection) return;
    const scheduledTime = scheduledStart ? new Date(scheduledStart).getTime() : 0;
    if (scheduledStart && (!Number.isFinite(scheduledTime) || scheduledTime <= Date.now())) {
      setError("Choose a scheduled start time in the future, or clear the field to start now.");
      return;
    }
    const scheduledForMs = scheduledStart ? Math.trunc(scheduledTime).toString() : null;
    setChoosingDestination(true);
    setError("");
    try {
      const filename = inspection.suggestedFilename || filenameFromUrl(inspection.effectiveUrl);
      const category = matchingCategory(filename, inspection.contentType, settings.categories);
      const destination = settings.smartRouting && settings.defaultDownloadPath && category
        ? await invoke<string>("resolve_smart_destination", {
            defaultPath: settings.defaultDownloadPath,
            categoryFolder: category.folder,
            filename,
          })
        : await save({
            title: "Save download as",
            defaultPath: filename,
          });
      if (!destination) return;

      const sourceUrl = inspection.sourceUrl;
      const browserRequestId =
        reviewingBrowserRequest?.url === sourceUrl ? reviewingBrowserRequest.id : undefined;
      setUrl("");
      setInspection(null);
      setScheduledStart("");
      void runDownload(sourceUrl, destination, undefined, browserRequestId, scheduledForMs);
    } catch (cause) {
      setError(String(cause));
    } finally {
      setChoosingDestination(false);
    }
  }

  async function openTorrentFile() {
    try {
      const path = await open({ multiple: false, filters: [{ name: "BitTorrent", extensions: ["torrent"] }] });
      if (typeof path !== "string") return;
      const source = await invoke<string>("torrent_file_url", { path });
      const result = await invoke<TorrentInspection>("inspect_torrent_source", { source, settings });
      setReviewingBrowserRequest(null);
      setTorrentInspection(result);
      setSourceMode("torrent");
    } catch (cause) { setError(String(cause)); }
  }

  async function chooseTorrentDestination() {
    if (!torrentInspection) return;
    setChoosingDestination(true);
    setError("");
    try {
      const destination = torrentDestination || await open({
            title: "Choose a folder for this torrent",
            directory: true,
            multiple: false,
            defaultPath: settings.defaultDownloadPath || undefined,
          });
      if (typeof destination !== "string") return;
      const selected = torrentInspection;
      setUrl("");
      setTorrentInspection(null);
      setSourceMode("torrent");
      void runTorrentDownload(selected.sourceUrl, destination, selected.name, reviewingBrowserRequest?.id, true, [...selectedTorrentFiles].sort((a, b) => a - b));
    } catch (cause) {
      setError(String(cause));
    } finally {
      setChoosingDestination(false);
    }
  }

  async function browseTorrentDestination() {
    const destination = await open({ title: "Choose torrent download folder", directory: true, multiple: false, defaultPath: torrentDestination || undefined });
    if (typeof destination === "string") setTorrentDestination(destination);
    setTorrentMenu(null);
  }

  async function loadMagnetFileList() {
    if (!torrentInspection || !torrentPrivacyConfirmed) return;
    setTorrentMetadataBusy(true);
    setError("");
    try {
      const result = await invoke<TorrentInspection>("preview_magnet_files", { source: torrentInspection.sourceUrl, settings, privacyConfirmed: true });
      setTorrentInspection(result);
    } catch (cause) { setError(String(cause)); }
    finally { setTorrentMetadataBusy(false); }
  }

  async function chooseMediaDestination() {
    if (!mediaInspection) return;
    setChoosingDestination(true);
    setError("");
    try {
      const categoryName = mediaQuality.startsWith("audio-") ? "Audio" : "Video";
      const category = settings.categories.find((entry) => entry.name === categoryName);
      const destination = settings.smartRouting && settings.defaultDownloadPath && category
        ? await invoke<string>("resolve_category_directory", {
            defaultPath: settings.defaultDownloadPath,
            categoryFolder: category.folder,
          })
        : await open({
            title: "Choose a folder for this media download",
            directory: true,
            multiple: false,
            defaultPath: settings.defaultDownloadPath || undefined,
          });
      if (typeof destination !== "string") return;
      const metadata = mediaInspection;
      setUrl("");
      setMediaInspection(null);
      setSourceMode("auto");
      void runMediaDownload(
        metadata.sourceUrl,
        destination,
        metadata.title,
        mediaQuality,
      );
    } catch (cause) {
      setError(String(cause));
    } finally {
      setChoosingDestination(false);
    }
  }

  async function chooseDefaultDownloadFolder() {
    setError("");
    try {
      const selected = await open({
        title: "Choose the default download folder",
        directory: true,
        multiple: false,
        defaultPath: settings.defaultDownloadPath || undefined,
      });
      if (typeof selected === "string") {
        setSettings((current) => ({ ...current, defaultDownloadPath: selected }));
      }
    } catch (cause) {
      setError(`Could not choose the default download folder: ${String(cause)}`);
    }
  }

  function updateCategory(index: number, patch: Partial<CategoryRule>) {
    setSettings((current) => ({
      ...current,
      categories: current.categories.map((category, categoryIndex) =>
        categoryIndex === index ? { ...category, ...patch } : category,
      ),
    }));
  }

  function addCategory() {
    setSettings((current) => ({
      ...current,
      categories: [
        ...current.categories,
        { name: "Custom", folder: "Custom", extensions: [".bin"], mimePrefixes: ["application/octet-stream"] },
      ],
    }));
  }

  function removeCategory(index: number) {
    setSettings((current) => ({
      ...current,
      categories: current.categories.filter((_, categoryIndex) => categoryIndex !== index),
    }));
  }

  function acceptClipboardCandidate() {
    if (!clipboardCandidate) return;
    setUrl(clipboardCandidate.url);
    setInspection(null);
    setMediaInspection(null);
    setTorrentInspection(null);
    setSourceMode(clipboardCandidate.kind === "media" ? "media" : clipboardCandidate.kind === "torrent" ? "torrent" : "auto");
    setClipboardCandidate(null);
    window.setTimeout(() => urlInputRef.current?.focus(), 0);
  }

  async function runDownload(
    sourceUrl: string,
    destination: string,
    existingId?: string,
    browserRequestId?: string,
    scheduledForMs: string | null = null,
  ) {
    await recoveryGate.current?.promise;
    const executionSettings = latestSettings.current;
    const id = existingId ?? browserRequestId ?? createTaskId();
    pendingCancellations.current.delete(id);
    if (nextQueueSequence.current > MAX_QUEUE_SEQUENCE) {
      setError("The durable queue sequence is exhausted; remove old entries and restart QuiverDL.");
      return;
    }
    const queueSequence = nextQueueSequence.current.toString();
    nextQueueSequence.current += 1n;
    const queuedAtMs = Date.now().toString();
    const item: DownloadItem = {
      id,
      name: destinationName(destination),
      url: sourceUrl,
      destination,
      status: queueStatus({ scheduledForMs }, executionSettings),
      downloadedBytes: 0n,
      totalBytes: null,
      recoverable: false,
      queuedAtMs,
      scheduledForMs,
      queueSequence,
      completedAtMs: null,
      kind: "direct",
    };
    setDownloads((current) =>
      current.some((existing) => existing.id === id)
        ? current.map((existing) => (existing.id === existingId ? item : existing))
        : [item, ...current],
    );
    setFilter("all");
    const { recoverable: _recoverable, ...storedItem } = item;
    const storedStartingItem: StoredDownload = {
      ...storedItem,
      downloadedBytes: storedItem.downloadedBytes.toString(),
      totalBytes: storedItem.totalBytes?.toString() ?? null,
    };
    const currentSnapshot = latestSnapshot.current ?? {
      schemaVersion: APP_STATE_SCHEMA_VERSION,
      settings: executionSettings,
      downloads: downloads.map(({ recoverable: _recoverable, ...download }) => ({
        ...download,
        downloadedBytes: download.downloadedBytes.toString(),
        totalBytes: download.totalBytes?.toString() ?? null,
      })),
    };
    const snapshot: AppSnapshot = {
      schemaVersion: APP_STATE_SCHEMA_VERSION,
      settings: executionSettings,
      downloads: currentSnapshot.downloads.some((download) => download.id === item.id)
        ? currentSnapshot.downloads.map((download) =>
            download.id === item.id ? storedStartingItem : download,
          )
        : [storedStartingItem, ...currentSnapshot.downloads],
    };
    try {
      await persistSnapshotNow(snapshot);
    } catch (cause) {
      if (browserRequestId) {
        setDownloads((current) => current.filter((existing) => existing.id !== item.id));
        setError(`Could not durably queue the browser download: ${String(cause)}`);
      } else {
        updateDownload(item.id, {
          status: "failed",
          error: `Could not durably queue the download: ${String(cause)}`,
        });
        setError(`Could not durably queue the download: ${String(cause)}`);
      }
      return;
    }

    if (!(await registerDownload(item, executionSettings))) return;

    if (browserRequestId) {
      try {
        await invoke("acknowledge_browser_request", { id: browserRequestId });
        setBrowserRequests((current) =>
          current.filter((request) => request.id !== browserRequestId),
        );
        setReviewingBrowserRequest(null);
      } catch (cause) {
        setError(`The download is safely queued, but its browser request remains: ${String(cause)}`);
      }
    }

    void executeDownload(item, executionSettings);
  }

  async function registerDownload(item: DownloadItem, executionSettings: AppSettings) {
    if (pendingCancellations.current.has(item.id)) {
      pendingCancellations.current.delete(item.id);
      updateDownload(item.id, { status: "cancelled", error: undefined });
      return false;
    }
    try {
      await invoke("register_download", {
        taskId: item.id,
        queueMode: executionSettings.queueMode,
        queueSequence: item.queueSequence,
        scheduledForMs: item.scheduledForMs,
      });
      registeredDownloads.current.add(item.id);
      if (pendingCancellations.current.has(item.id)) {
        await invoke("discard_registered_download", { taskId: item.id });
        registeredDownloads.current.delete(item.id);
        pendingCancellations.current.delete(item.id);
        updateDownload(item.id, { status: "cancelled", error: undefined });
        return false;
      }
      return true;
    } catch (cause) {
      registeredDownloads.current.delete(item.id);
      void invoke("discard_registered_download", { taskId: item.id });
      pendingCancellations.current.delete(item.id);
      updateDownload(item.id, { status: "failed", error: String(cause) });
      return false;
    }
  }

  async function executeDownload(item: DownloadItem, executionSettings: AppSettings) {
    const { id, url: sourceUrl, destination } = item;
    updateDownload(id, {
      status: queueStatus(item, executionSettings),
      error: undefined,
      recoverable: false,
    });

    let dueTimer: number | null = null;
    const clearDueTimer = () => {
      if (dueTimer !== null) {
        window.clearTimeout(dueTimer);
        dueTimer = null;
      }
    };
    if (executionSettings.queueMode === "sequential" && item.scheduledForMs !== null) {
      const refreshDueStatus = () => {
        const remainingMs = Number(item.scheduledForMs) - Date.now();
        if (remainingMs <= 0) {
          updateDownload(id, (current) =>
            current.status === "scheduled" ? { status: "queued" } : {},
          );
          return;
        }
        dueTimer = window.setTimeout(refreshDueStatus, Math.min(remainingMs, 30_000));
      };
      refreshDueStatus();
    }

    const onEvent = new Channel<DownloadProgress>();
    onEvent.onmessage = (message) => {
      clearDueTimer();
      updateDownload(id, (current) => ({
        status:
          current.status === "paused" ||
          current.status === "cancelling" ||
          current.status === "cancelled"
            ? current.status
            : message.status,
        downloadedBytes: BigInt(message.downloadedBytes),
        totalBytes: message.totalBytes === null ? null : BigInt(message.totalBytes),
      }));
    };

    try {
      const summary = await invoke<DownloadSummary>("start_download", {
        taskId: id,
        url: sourceUrl,
        destination,
        settings: executionSettings,
        scheduledForMs: item.scheduledForMs,
        onEvent,
      });
      updateDownload(id, {
        status: "completed",
        downloadedBytes: BigInt(summary.bytesWritten),
        totalBytes: BigInt(summary.bytesWritten),
        sha256: summary.sha256,
        resumed: summary.resumed,
        completedAtMs: Date.now().toString(),
      });
      if (executionSettings.notifications) {
        void notifyCompleted(destinationName(destination));
      }
    } catch (cause) {
      const failure = String(cause);
      updateDownload(id, (current) => {
        const cancelled =
          current.status === "cancelling" || failure.toLowerCase().includes("cancelled");
        return {
          status: cancelled ? "cancelled" : "failed",
          error: cancelled ? undefined : failure,
        };
      });
    } finally {
      clearDueTimer();
      registeredDownloads.current.delete(id);
      void invoke("discard_registered_download", { taskId: id });
    }
  }

  async function runMediaDownload(
    sourceUrl: string,
    destinationDirectory: string,
    title: string,
    quality: string,
    existingId?: string,
  ) {
    await recoveryGate.current?.promise;
    const executionSettings = latestSettings.current;
    const id = existingId ?? createTaskId();
    pendingCancellations.current.delete(id);
    if (nextQueueSequence.current > MAX_QUEUE_SEQUENCE) {
      setError("The durable queue sequence is exhausted; remove old entries and restart QuiverDL.");
      return;
    }
    const queueSequence = nextQueueSequence.current.toString();
    nextQueueSequence.current += 1n;
    const item: DownloadItem = {
      id,
      name: title,
      url: sourceUrl,
      destination: destinationDirectory,
      status: queueStatus({ scheduledForMs: null }, executionSettings),
      downloadedBytes: 0n,
      totalBytes: null,
      recoverable: false,
      queuedAtMs: Date.now().toString(),
      scheduledForMs: null,
      queueSequence,
      completedAtMs: null,
      kind: "media",
      mediaQuality: quality,
    };
    setDownloads((current) => current.some((entry) => entry.id === id)
      ? current.map((entry) => entry.id === existingId ? item : entry)
      : [item, ...current]);
    setFilter("all");
    const { recoverable: _recoverable, ...storedItem } = item;
    const serialized: StoredDownload = {
      ...storedItem,
      downloadedBytes: "0",
      totalBytes: null,
    };
    const currentSnapshot = latestSnapshot.current ?? {
      schemaVersion: APP_STATE_SCHEMA_VERSION,
      settings: executionSettings,
      downloads: [],
    };
    try {
      await persistSnapshotNow({
        schemaVersion: APP_STATE_SCHEMA_VERSION,
        settings: executionSettings,
        downloads: currentSnapshot.downloads.some((entry) => entry.id === item.id)
          ? currentSnapshot.downloads.map((entry) => entry.id === item.id ? serialized : entry)
          : [serialized, ...currentSnapshot.downloads],
      });
    } catch (cause) {
      updateDownload(item.id, { status: "failed", error: `Could not durably queue the media download: ${String(cause)}` });
      return;
    }
    if (!(await registerDownload(item, executionSettings))) return;
    void executeMediaDownload(item, executionSettings);
  }

  async function executeMediaDownload(item: DownloadItem, executionSettings: AppSettings) {
    updateDownload(item.id, {
      status: queueStatus(item, executionSettings),
      error: undefined,
      recoverable: false,
    });
    const onEvent = new Channel<MediaProgress>();
    onEvent.onmessage = (message) => {
      updateDownload(item.id, (current) => ({
        status: current.status === "cancelling" || current.status === "cancelled"
          ? current.status
          : message.status,
        downloadedBytes: BigInt(message.downloadedBytes),
        totalBytes: message.totalBytes === null ? current.totalBytes : BigInt(message.totalBytes),
      }));
    };
    try {
      const summary = await invoke<MediaSummary>("start_media_download", {
        request: {
          taskId: item.id,
          url: item.url,
          destinationDirectory: item.destination,
          quality: item.mediaQuality ?? "best",
          settings: executionSettings,
        },
        onEvent,
      });
      const bytesWritten = BigInt(summary.bytesWritten);
      updateDownload(item.id, {
        status: "completed",
        destination: summary.destination,
        name: destinationName(summary.destination),
        downloadedBytes: bytesWritten,
        totalBytes: bytesWritten,
        completedAtMs: Date.now().toString(),
      });
      if (executionSettings.notifications) void notifyCompleted(destinationName(summary.destination));
    } catch (cause) {
      const failure = String(cause);
      updateDownload(item.id, (current) => {
        const cancelled = current.status === "cancelling" || failure.toLowerCase().includes("cancelled");
        return { status: cancelled ? "cancelled" : "failed", error: cancelled ? undefined : failure };
      });
    } finally {
      registeredDownloads.current.delete(item.id);
      void invoke("discard_registered_download", { taskId: item.id });
    }
  }

  async function runTorrentDownload(
    sourceUrl: string,
    destinationDirectory: string,
    title: string,
    existingId?: string,
    privacyConfirmed = false,
    selectedFiles?: number[],
  ) {
    await recoveryGate.current?.promise;
    const executionSettings = latestSettings.current;
    if (!privacyConfirmed) {
      setError("Review and confirm the BitTorrent privacy disclosure before starting.");
      return;
    }
    const id = existingId ?? createTaskId();
    pendingCancellations.current.delete(id);
    if (nextQueueSequence.current > MAX_QUEUE_SEQUENCE) {
      setError("The durable queue sequence is exhausted; remove old entries and restart QuiverDL.");
      return;
    }
    const queueSequence = nextQueueSequence.current.toString();
    nextQueueSequence.current += 1n;
    const item: DownloadItem = {
      id,
      name: title,
      url: sourceUrl,
      destination: destinationDirectory,
      status: queueStatus({ scheduledForMs: null }, executionSettings),
      downloadedBytes: 0n,
      totalBytes: null,
      recoverable: false,
      queuedAtMs: Date.now().toString(),
      scheduledForMs: null,
      queueSequence,
      completedAtMs: null,
      kind: "torrent",
      torrentSelection: selectedFiles,
    };
    setDownloads((current) => current.some((entry) => entry.id === id)
      ? current.map((entry) => entry.id === id ? item : entry)
      : [item, ...current]);
    setFilter("all");
    const { recoverable: _recoverable, ...storedItem } = item;
    const serialized: StoredDownload = {
      ...storedItem,
      downloadedBytes: "0",
      totalBytes: null,
    };
    const currentSnapshot = latestSnapshot.current ?? {
      schemaVersion: APP_STATE_SCHEMA_VERSION,
      settings: executionSettings,
      downloads: [],
    };
    try {
      await persistSnapshotNow({
        schemaVersion: APP_STATE_SCHEMA_VERSION,
        settings: executionSettings,
        downloads: currentSnapshot.downloads.some((entry) => entry.id === item.id)
          ? currentSnapshot.downloads.map((entry) => entry.id === item.id ? serialized : entry)
          : [serialized, ...currentSnapshot.downloads],
      });
    } catch (cause) {
      updateDownload(item.id, { status: "failed", error: `Could not durably queue the torrent: ${String(cause)}` });
      return;
    }
    if (!(await registerDownload(item, executionSettings))) return;
    if (reviewingBrowserRequest?.id === id) {
      await invoke("acknowledge_browser_request", { id });
      setReviewingBrowserRequest(null);
      setBrowserRequests((current) => current.filter((request) => request.id !== id));
    }
    void executeTorrentDownload(item, executionSettings, privacyConfirmed);
  }

  async function executeTorrentDownload(
    item: DownloadItem,
    executionSettings: AppSettings,
    privacyConfirmed: boolean,
  ) {
    updateDownload(item.id, {
      status: queueStatus(item, executionSettings),
      error: undefined,
      recoverable: false,
    });
    const onEvent = new Channel<TorrentProgress>();
    onEvent.onmessage = (message) => {
      updateDownload(item.id, (current) => ({
        status: current.status === "cancelling" || current.status === "cancelled"
          ? current.status
          : message.status,
        name: message.name ?? current.name,
        downloadedBytes: BigInt(message.downloadedBytes),
        totalBytes: message.totalBytes === null ? current.totalBytes : BigInt(message.totalBytes),
      }));
    };
    try {
      const summary = await invoke<TorrentSummary>("start_torrent_download", {
        request: {
          taskId: item.id,
          source: item.url,
          destinationDirectory: item.destination,
          settings: executionSettings,
          privacyConfirmed,
          selectedFiles: item.torrentSelection ?? null,
        },
        onEvent,
      });
      const bytesWritten = BigInt(summary.bytesWritten);
      updateDownload(item.id, {
        status: "completed",
        destination: summary.destination,
        name: summary.name,
        downloadedBytes: bytesWritten,
        totalBytes: bytesWritten,
        completedAtMs: Date.now().toString(),
      });
      if (executionSettings.notifications) void notifyCompleted(summary.name);
    } catch (cause) {
      const failure = String(cause);
      updateDownload(item.id, (current) => {
        const cancelled = current.status === "cancelling" || failure.toLowerCase().includes("cancelled");
        return { status: cancelled ? "cancelled" : "failed", error: cancelled ? undefined : failure };
      });
    } finally {
      registeredDownloads.current.delete(item.id);
      void invoke("discard_registered_download", { taskId: item.id });
    }
  }

  function retryDownload(item: DownloadItem) {
    if (item.kind === "media") {
      void runMediaDownload(
        item.url,
        item.destination,
        item.name,
        item.mediaQuality ?? "best",
        item.id,
      );
      return;
    }
    if (item.kind === "torrent") {
      setUrl(item.url);
      setSourceMode("torrent");
      setTorrentInspection(null);
      setError("Inspect this torrent again and confirm its privacy disclosure before retrying.");
      urlInputRef.current?.focus();
      return;
    }
    void runDownload(item.url, item.destination, item.id, undefined, null);
  }

  function reviewBrowserRequest(request: BrowserRequest) {
    setUrl(request.url);
    setInspection(null);
    setMediaInspection(null);
    setTorrentInspection(null);
    setSourceMode("auto");
    setFilter("all");
    setReviewingBrowserRequest(request);
  }

  async function revealBrowserBridge() {
    try {
      setBridgeInfo(await invoke<BrowserBridgeInfo>("get_browser_bridge_info"));
    } catch (cause) {
      setError(String(cause));
    }
  }

  async function storeProxyCredentials() {
    setProxyCredentialBusy(true);
    setError("");
    try {
      await invoke("save_proxy_credentials", {
        endpoint: proxyDraft.proxyUrl,
        username: proxyDraft.proxyUsername,
        password: proxyPassword,
      });
      setProxyPassword("");
      setProxyCredentialsPresent(true);
    } catch (cause) {
      setError(String(cause));
    } finally {
      setProxyCredentialBusy(false);
    }
  }

  async function applyProxyConfiguration() {
    setProxyCredentialBusy(true);
    setError("");
    const nextSettings: AppSettings = { ...settings, ...proxyDraft };
    try {
      await invoke("validate_proxy_configuration", { settings: nextSettings });
      setSettings(nextSettings);
    } catch (cause) {
      setError(String(cause));
    } finally {
      setProxyCredentialBusy(false);
    }
  }

  async function removeProxyCredentials() {
    setProxyCredentialBusy(true);
    setError("");
    try {
      await invoke("clear_proxy_credentials");
      setProxyPassword("");
      setProxyCredentialsPresent(false);
    } catch (cause) {
      setError(String(cause));
    } finally {
      setProxyCredentialBusy(false);
    }
  }

  async function controlDownload(item: DownloadItem, action: "pause" | "resume" | "cancel") {
    setError("");
    if (action === "cancel" && !registeredDownloads.current.has(item.id)) {
      pendingCancellations.current.add(item.id);
      updateDownload(item.id, { status: "cancelling", error: undefined });
      return;
    }
    if (item.kind === "media") {
      if (action !== "cancel") return;
      updateDownload(item.id, { status: "cancelling", error: undefined });
      const results = await Promise.allSettled([
        invoke("control_download", { taskId: item.id, action: "cancel" }),
        invoke("cancel_media_download", { taskId: item.id }),
      ]);
      if (results.every((result) => result.status === "rejected")) {
        setError(String((results[0] as PromiseRejectedResult).reason));
      }
      return;
    }
    if (item.kind === "torrent") {
      if (action === "cancel") {
        updateDownload(item.id, { status: "cancelling", error: undefined });
        const results = await Promise.allSettled([
          invoke("control_download", { taskId: item.id, action: "cancel" }),
          invoke("control_torrent_download", { taskId: item.id, action: "cancel" }),
        ]);
        if (results.every((result) => result.status === "rejected")) {
          setError(String((results[0] as PromiseRejectedResult).reason));
        }
        return;
      }
      try {
        await invoke("control_torrent_download", { taskId: item.id, action });
        updateDownload(item.id, {
          status: action === "pause" ? "paused" : action === "resume" ? "downloading" : "cancelling",
        });
      } catch (cause) {
        setError(String(cause));
      }
      return;
    }
    try {
      await invoke("control_download", { taskId: item.id, action });
      updateDownload(item.id, {
        status:
          action === "pause"
            ? "paused"
            : action === "resume"
              ? "downloading"
              : "cancelling",
      });
    } catch (cause) {
      setError(String(cause));
    }
  }

  function removeDownload(id: string) {
    setDownloads((current) => current.filter((item) => item.id !== id));
  }

  function clearCompletedHistory() {
    if (counts.completed === 0) return;
    if (!window.confirm(t("clearCompletedConfirm"))) return;
    setDownloads((current) => current.filter((item) => item.status !== "completed"));
    setHistoryQuery("");
  }

  async function checkForUpdates(reportCurrent: boolean) {
    if (!UPDATER_ENABLED || updaterOperation.current !== null) return;
    updaterOperation.current = "check";
    setUpdateChecking(true);
    if (reportCurrent) {
      setUpdateError("");
      setUpdateStatus(t("checkingForUpdates"));
    }
    try {
      const update = await check({ timeout: 20_000 });
      setUpdateError("");
      if (!update) {
        if (availableUpdate.current) await availableUpdate.current.close();
        availableUpdate.current = null;
        setAvailableUpdateVersion(null);
        setUpdateDownloaded(false);
        setUpdateProgress(null);
        if (reportCurrent) setUpdateStatus(t("upToDate"));
        return;
      }
      if (availableUpdate.current?.version === update.version) {
        await update.close();
        if (reportCurrent) setUpdateStatus(updateDownloaded ? t("updateReady") : "");
        return;
      }
      if (availableUpdate.current) await availableUpdate.current.close();
      availableUpdate.current = update;
      setAvailableUpdateVersion(update.version);
      setUpdateDownloaded(false);
      setUpdateProgress(null);
      setUpdateStatus("");
    } catch {
      if (reportCurrent) setUpdateStatus("");
      if (reportCurrent) setUpdateError(t("updateCheckFailed"));
    } finally {
      updaterOperation.current = null;
      setUpdateChecking(false);
    }
  }

  async function downloadAvailableUpdate() {
    const update = availableUpdate.current;
    if (!update || updaterOperation.current !== null) return;
    if (!window.confirm(t("downloadUpdateConfirm"))) return;
    setUpdateError("");
    updaterOperation.current = "action";
    setUpdateBusy(true);
    setUpdateStatus(t("downloadingUpdate"));
    setUpdateProgress(null);
    let downloadedBytes = 0;
    let contentLength: number | undefined;
    try {
      await update.download((event: DownloadEvent) => {
        if (event.event === "Started") {
          contentLength = event.data.contentLength;
        } else if (event.event === "Progress") {
          downloadedBytes += event.data.chunkLength;
          if (contentLength && contentLength > 0) {
            setUpdateProgress(Math.min(100, Math.round((downloadedBytes / contentLength) * 100)));
          }
        } else {
          setUpdateProgress(100);
        }
      });
      setUpdateDownloaded(true);
      setUpdateStatus(t("updateReady"));
    } catch {
      setUpdateError(t("updateDownloadFailed"));
      setUpdateStatus("");
    } finally {
      updaterOperation.current = null;
      setUpdateBusy(false);
    }
  }

  async function flushAppStateForUpdate() {
    if (saveTimer.current !== null) {
      window.clearTimeout(saveTimer.current);
      saveTimer.current = null;
    }
    while (saveInFlight.current) {
      await new Promise((resolve) => window.setTimeout(resolve, 25));
    }
    if (!latestSnapshot.current) throw new Error("App state is not ready");
    await invoke("save_app_state", { snapshot: latestSnapshot.current });
  }

  async function installDownloadedUpdate() {
    const update = availableUpdate.current;
    if (!update || !updateDownloaded || updaterOperation.current !== null) return;
    if (downloads.some((item) => ACTIVE_STATUSES.has(item.status))) {
      setUpdateError(t("updateBlocked"));
      return;
    }
    if (!window.confirm(t("restartUpdateConfirm"))) return;
    setUpdateError("");
    updaterOperation.current = "action";
    setUpdateBusy(true);
    let gateHeld = false;
    let installed = false;
    try {
      await invoke("begin_update_install");
      gateHeld = true;
      await flushAppStateForUpdate();
      await update.install();
      installed = true;
      await relaunch();
    } catch {
      if (installed) {
        setUpdateError(t("updateRestartFailed"));
        setUpdateStatus(t("updateRestartFailed"));
      } else {
        if (gateHeld) await invoke("cancel_update_install").catch(() => undefined);
        setUpdateError(t("updateInstallFailed"));
        updaterOperation.current = null;
        setUpdateBusy(false);
      }
    }
  }

  return (
    <div className="app-shell">
      <div className="app-titlebar"
        onMouseDown={(event) => {
          if (event.button !== 0 || (event.target as HTMLElement).closest("button")) return;
          if (event.detail === 2) void getCurrentWindow().toggleMaximize();
          else void getCurrentWindow().startDragging();
        }}>
        <div className="app-titlebar-name"><img src={quiverLogo} alt="" draggable={false} /><span>QuiverDL</span></div>
            <div className="window-controls">
              <button type="button" aria-label="Minimize QuiverDL" onClick={() => void getCurrentWindow().minimize()}>
                <svg aria-hidden="true" viewBox="0 0 12 12"><path d="M2 6.5h8" /></svg>
              </button>
              <button type="button" aria-label="Maximize or restore QuiverDL" onClick={() => void getCurrentWindow().toggleMaximize()}>
                <svg aria-hidden="true" viewBox="0 0 12 12"><rect x="2.25" y="2.25" width="7.5" height="7.5" rx=".4" /></svg>
              </button>
              <button className="window-close" type="button" aria-label="Close QuiverDL" onClick={() => void getCurrentWindow().close()}>
                <svg aria-hidden="true" viewBox="0 0 12 12"><path d="m2.5 2.5 7 7m0-7-7 7" /></svg>
              </button>
            </div>
      </div>
      <aside className="sidebar">
        <div
          className="brand"
          data-tauri-drag-region
          onDoubleClick={() => void getCurrentWindow().toggleMaximize()}
        >
          <img className="brand-mark" src={quiverLogo} alt="" aria-hidden="true" draggable={false} data-tauri-drag-region />
          <span data-tauri-drag-region>QuiverDL</span>
        </div>
        <nav aria-label="Download filters">
          <FilterButton active={filter === "all"} count={downloads.length} label={t("downloads")} onClick={() => setFilter("all")} />
          <FilterButton active={filter === "active"} count={counts.active} label={t("active")} onClick={() => setFilter("active")} />
          <FilterButton active={filter === "completed"} count={counts.completed} label={t("completed")} onClick={() => setFilter("completed")} />
          <FilterButton active={filter === "failed"} count={counts.failed} label={t("attention")} onClick={() => setFilter("failed")} />
        </nav>
        <div className="privacy-note">
          {t("privateDesign")}
          <small>{t("noTelemetry")}</small>
        </div>
        <button className="sidebar-settings-button" type="button" onClick={(event) => openSettings(event.currentTarget)}>
          <span aria-hidden="true">⚙</span>
          {t("settings")}
        </button>
        {settingsOpen && (
          <div className="modal-backdrop" role="presentation" onMouseDown={(event) => {
            if (event.target === event.currentTarget) closeSettings();
          }}>
            <section ref={settingsModalRef} className="settings-modal" role="dialog" aria-modal="true" aria-labelledby="settings-title" tabIndex={-1}>
              <div className="settings-modal-header">
                <div>
                  <p className="eyebrow">QUIVERDL CONTROL CENTER</p>
                  <h2 id="settings-title">{t("settings")}</h2>
                </div>
                <button type="button" aria-label="Close settings" autoFocus onClick={closeSettings}>×</button>
              </div>
              <div className="settings-navigation">
                <label className="settings-search">
                  <span>{settings.language === "ar" ? "البحث في الإعدادات" : "Find a setting"}</span>
                  <input type="search" value={settingsSearch} onChange={(event) => setSettingsSearch(event.target.value)} placeholder={settings.language === "ar" ? "ابحث عن السرعة أو المجلد أو الوكيل…" : "Search speed, folder, proxy…"} />
                </label>
                <div className="settings-tabs" role="tablist" aria-label="Settings categories">
                  {SETTINGS_TABS.map((tab, index) => <button key={tab.id} id={`settings-tab-${tab.id}`} role="tab" type="button" aria-selected={settingsTab === tab.id && !settingsSearch.trim()} aria-controls="settings-content" tabIndex={settingsTab === tab.id ? 0 : -1}
                    onClick={() => { setSettingsTab(tab.id); setSettingsSearch(""); }}
                    onKeyDown={(event) => {
                      const delta = event.key === "ArrowRight" ? 1 : event.key === "ArrowLeft" ? -1 : 0;
                      const next = event.key === "Home" ? 0 : event.key === "End" ? SETTINGS_TABS.length - 1 : (index + delta + SETTINGS_TABS.length) % SETTINGS_TABS.length;
                      if (delta || event.key === "Home" || event.key === "End") {
                        event.preventDefault(); setSettingsTab(SETTINGS_TABS[next].id); setSettingsSearch("");
                        document.getElementById(`settings-tab-${SETTINGS_TABS[next].id}`)?.focus();
                      }
                    }}>{settings.language === "ar" ? tab.ar : tab.en}</button>)}
                </div>
              </div>
              <div id="settings-content" role={settingsSearch.trim() ? "region" : "tabpanel"} aria-label={settingsSearch.trim() ? "Settings search results" : undefined} aria-labelledby={settingsSearch.trim() ? undefined : `settings-tab-${settingsTab}`} className="settings-panel settings-modal-body">
                {settingsSearch.trim() && <p role="status">{SETTINGS_TABS.filter((tab) => settingsMatches(tab.id)).length ? "Matching settings" : "No settings found. Try another search."}</p>}
          <SettingsSection title="Appearance and language" visible={settingsMatches("general")}>
          <fieldset className="settings-group theme-settings">
            <legend>Theme</legend>
            <label>
              Mode
              <select value={settings.theme} onChange={(event) => setSettings((current) => ({ ...current, theme: event.target.value as AppSettings["theme"] }))}>
                <option value="system">System</option>
                <option value="light">Light</option>
                <option value="dark">Dark</option>
              </select>
            </label>
            <span className="setting-label">Accent color</span>
            <div className="accent-presets" role="group" aria-label="Accent color presets">
              {THEME_ACCENTS.map((accent) => (
                <button
                  className={settings.accentColor?.toUpperCase() === accent.color ? "selected" : undefined}
                  type="button"
                  key={accent.color}
                  aria-label={`${accent.name} accent`}
                  aria-pressed={settings.accentColor?.toUpperCase() === accent.color}
                  title={accent.name}
                  style={{ backgroundColor: accent.color }}
                  onClick={() => setSettings((current) => ({ ...current, accentColor: accent.color }))}
                />
              ))}
            </div>
            <div className="accent-custom">
              <label>
                Custom
                <input
                  type="color"
                  value={settings.accentColor ?? "#62A7FF"}
                  onChange={(event) => setSettings((current) => ({ ...current, accentColor: event.target.value.toUpperCase() }))}
                />
              </label>
              <button type="button" onClick={() => setSettings((current) => ({ ...current, accentColor: null }))}>
                Use theme default
              </button>
            </div>
          </fieldset>
          <label>
            {t("language")}
            <select value={settings.language} onChange={(event) => setSettings((current) => ({ ...current, language: event.target.value as AppSettings["language"] }))}>
              <option value="en">English</option>
              <option value="ar">العربية</option>
            </select>
          </label>

          </SettingsSection>
          <SettingsSection title="Transfer settings" visible={settingsMatches("downloads")}>
          <label>
            {t("retryAttempts")}
            <input type="number" min={1} max={10} value={settings.retryAttempts} onChange={(event) => setSettings((current) => ({ ...current, retryAttempts: Math.max(1, Math.min(10, Number(event.target.value))) }))} />
          </label>
          <label>
            {t("connectionsDownload")}
            <input type="number" min={1} max={16} value={settings.maxSegments} onChange={(event) => setSettings((current) => ({ ...current, maxSegments: Math.max(1, Math.min(16, Number(event.target.value))) }))} />
          </label>
          <label>
            {t("connectionsServer")}
            <input type="number" min={1} max={32} value={settings.maxConnectionsPerHost} onChange={(event) => setSettings((current) => ({ ...current, maxConnectionsPerHost: Math.max(1, Math.min(32, Number(event.target.value))) }))} />
          </label>
          <label>
            Per-download limit (KiB/s, 0 = unlimited)
            <input type="number" min={0} step={128} value={settings.perDownloadSpeedLimitBps === null ? 0 : Math.round(settings.perDownloadSpeedLimitBps / 1024)} onChange={(event) => { const value = Math.max(0, Number(event.target.value)); setSettings((current) => ({ ...current, perDownloadSpeedLimitBps: value === 0 ? null : value * 1024 })); }} />
          </label>
          <label>
            Global limit (KiB/s, 0 = unlimited)
            <input type="number" min={0} step={128} value={settings.globalSpeedLimitBps === null ? 0 : Math.round(settings.globalSpeedLimitBps / 1024)} onChange={(event) => { const value = Math.max(0, Number(event.target.value)); setSettings((current) => ({ ...current, globalSpeedLimitBps: value === 0 ? null : value * 1024 })); }} />
          </label>
          <label>
            Queue mode
            <select
              value={settings.queueMode}
              onChange={(event) => setSettings((current) => ({
                ...current,
                queueMode: event.target.value as AppSettings["queueMode"],
              }))}
            >
              <option value="parallel">Parallel</option>
              <option value="sequential">Sequential (FIFO)</option>
            </select>
          </label>
          <small className="queue-help">
            Sequential mode starts one queued download at a time. Scheduled items join the queue when due.
          </small>

          </SettingsSection>
          <SettingsSection title="Notifications and history" visible={settingsMatches("general")}>
          <label className="checkbox-setting">
            <input type="checkbox" checked={settings.notifications} onChange={(event) => setSettings((current) => ({ ...current, notifications: event.target.checked }))} />
            {t("notifications")}
          </label>
          <label>
            {t("historyRetention")}
            <select
              value={settings.historyRetentionDays ?? "forever"}
              onChange={(event) => {
                const value = event.target.value;
                setSettings((current) => ({
                  ...current,
                  historyRetentionDays: value === "forever"
                    ? null
                    : Number(value) as 7 | 30 | 90,
                }));
              }}
            >
              <option value="forever">{t("keepForever")}</option>
              <option value="7">{t("keepSevenDays")}</option>
              <option value="30">{t("keepThirtyDays")}</option>
              <option value="90">{t("keepNinetyDays")}</option>
            </select>
          </label>
          <small className="queue-help">{t("historyRetentionHint")}</small>

          </SettingsSection>
          <SettingsSection title="Download folders" visible={settingsMatches("folders")}>
          <fieldset className="settings-group routing-settings">
            <legend>Capture & routing</legend>
            <label className="checkbox-setting">
              <input
                type="checkbox"
                checked={settings.smartRouting}
                onChange={(event) => setSettings((current) => ({
                  ...current,
                  smartRouting: event.target.checked,
                }))}
              />
              Sort downloads into category folders
            </label>
            <label>
              Default download folder
              <div className="path-picker-row">
                <input type="text" value={settings.defaultDownloadPath} readOnly placeholder="Choose a folder" />
                <button type="button" onClick={() => void chooseDefaultDownloadFolder()}>Browse</button>
              </div>
            </label>
            <div className="category-editor">
              <div className="category-heading">
                <strong>Smart categories</strong>
                <button type="button" onClick={addCategory} disabled={settings.categories.length >= 32}>Add category</button>
              </div>
              {settings.categories.map((category, index) => (
                <details className="category-card" key={index}><summary>{category.name || "New category"}</summary>
                  <label>
                    Name
                    <input value={category.name} onChange={(event) => updateCategory(index, { name: event.target.value })} />
                  </label>
                  <label>
                    Folder
                    <input value={category.folder} onChange={(event) => updateCategory(index, { folder: event.target.value })} />
                  </label>
                  <label className="category-wide">
                    Extensions (comma separated)
                    <input
                      value={category.extensions.join(", ")}
                      onChange={(event) => updateCategory(index, {
                        extensions: event.target.value.split(",").map((value) => value.trim().toLowerCase()).filter(Boolean),
                      })}
                    />
                  </label>
                  <label className="category-wide">
                    MIME prefixes (comma separated)
                    <input
                      value={category.mimePrefixes.join(", ")}
                      onChange={(event) => updateCategory(index, {
                        mimePrefixes: event.target.value.split(",").map((value) => value.trim().toLowerCase()).filter(Boolean),
                      })}
                    />
                  </label>
                  {settings.categories.length > 1 && (
                    <button className="remove-category" type="button" onClick={() => removeCategory(index)}>Remove</button>
                  )}
                </details>
              ))}
            </div>
          </fieldset>

          </SettingsSection>
          <SettingsSection title="Media downloads" visible={settingsMatches("media")}>
          <fieldset className="settings-group media-engine-settings">
            <legend>Media engine</legend>
            <label>
              Python executable (optional)
              <input
                type="text"
                value={settings.mediaPythonPath}
                placeholder="Auto-detect python3 / python / py"
                onChange={(event) => setSettings((current) => ({ ...current, mediaPythonPath: event.target.value }))}
              />
            </label>
            <small className="credential-status">Media downloads use the yt-dlp Python API. Install with <code>python -m pip install -U yt-dlp</code>; FFmpeg is required for merged video and audio conversion.</small>
          </fieldset>

          </SettingsSection>
          <SettingsSection title="Connection and proxy" visible={settingsMatches("network")}>
          <fieldset className="proxy-settings">
            <legend>Proxy</legend>
            <label>
              Routing
              <select
                value={proxyDraft.proxyMode}
                onChange={(event) => {
                  setProxyPassword("");
                  setProxyDraft((current) => ({
                    ...current,
                    proxyMode: event.target.value as AppSettings["proxyMode"],
                  }));
                }}
              >
                <option value="disabled">Direct connection</option>
                <option value="system">System proxy</option>
                <option value="custom">Custom proxy</option>
              </select>
            </label>
            {proxyDraft.proxyMode === "custom" && (
              <div className="proxy-fields">
                <label>
                  Proxy URL
                  <input
                    type="url"
                    value={proxyDraft.proxyUrl}
                    placeholder="http://proxy.example:8080"
                    autoComplete="off"
                    onChange={(event) => {
                      setProxyPassword("");
                      setProxyCredentialsPresent(false);
                      setProxyDraft((current) => ({ ...current, proxyUrl: event.target.value }));
                    }}
                  />
                </label>
                <label>
                  Bypass list
                  <input
                    type="text"
                    value={proxyDraft.proxyBypass}
                    placeholder="localhost, .internal.example"
                    autoComplete="off"
                    onChange={(event) =>
                      setProxyDraft((current) => ({ ...current, proxyBypass: event.target.value }))
                    }
                  />
                </label>
                <label>
                  Username (optional)
                  <input
                    type="text"
                    value={proxyDraft.proxyUsername}
                    autoComplete="username"
                    onChange={(event) => {
                      setProxyPassword("");
                      setProxyCredentialsPresent(false);
                      setProxyDraft((current) => ({
                        ...current,
                        proxyUsername: event.target.value,
                      }));
                    }}
                  />
                </label>
                {proxyDraft.proxyUsername && (
                  <>
                    <label>
                      Password
                      <input
                        type="password"
                        value={proxyPassword}
                        autoComplete="current-password"
                        placeholder={proxyCredentialsPresent ? "Stored securely" : "Required"}
                        onChange={(event) => setProxyPassword(event.target.value)}
                      />
                    </label>
                    <div className="proxy-credential-actions">
                      <button
                        type="button"
                        disabled={proxyCredentialBusy || !proxyPassword}
                        onClick={() => void storeProxyCredentials()}
                      >
                        Save password securely
                      </button>
                      {proxyCredentialsPresent && (
                        <button
                          type="button"
                          disabled={proxyCredentialBusy}
                          onClick={() => void removeProxyCredentials()}
                        >
                          Remove
                        </button>
                      )}
                    </div>
                    <small className="credential-status" role="status">
                      {proxyCredentialsPresent
                        ? "Password stored in your operating-system credential store."
                        : "Passwords are never saved in QuiverDL settings."}
                    </small>
                  </>
                )}
              </div>
            )}
            <button
              className="apply-proxy"
              type="button"
              disabled={proxyCredentialBusy}
              onClick={() => void applyProxyConfiguration()}
            >
              Apply proxy settings
            </button>
            <small className="credential-status">
              Active: {settings.proxyMode === "disabled"
                ? "direct connection"
                : settings.proxyMode === "system"
                  ? "system proxy"
                  : settings.proxyUrl}
            </small>
          </fieldset>

          </SettingsSection>
          <SettingsSection title="Firefox integration" visible={settingsMatches("integration")}>
          <button className="bridge-button" type="button" onClick={() => void revealBrowserBridge()}>
            Check browser integration
          </button>

          </SettingsSection>
          <SettingsSection title="App updates" visible={settingsMatches("updates")}>
          {UPDATER_ENABLED && (
            <div className="updater-settings">
              <button
                className="bridge-button"
                type="button"
                disabled={updateBusy || updateChecking}
                onClick={() => void checkForUpdates(true)}
              >
                {updateChecking ? t("checkingForUpdates") : t("checkForUpdates")}
              </button>
              {updateStatus && <small role="status">{updateStatus}</small>}
            </div>
          )}

          </SettingsSection>
          <SettingsSection title="Connection status" visible={settingsMatches("integration")}>
          {bridgeInfo && (
            <div className="bridge-secret">
              <strong>{bridgeInfo.connected ? "Firefox ready" : "Browser integration needs attention"}</strong>
              <span>{bridgeInfo.message}</span>
            </div>
          )}

          </SettingsSection>

          <SettingsSection title="Clipboard capture" visible={settingsMatches("integration")}>
            <label className="checkbox-setting">
              <input
                type="checkbox"
                checked={settings.clipboardMonitoring}
                onChange={(event) => setSettings((current) => ({
                  ...current,
                  clipboardMonitoring: event.target.checked,
                }))}
              />
              Monitor copied download links
            </label>
            <small className="queue-help">Only URL-shaped clipboard text is inspected. Clipboard contents never leave this device.</small>

          </SettingsSection>
          {settingsMatches("updates") && !UPDATER_ENABLED && <p>Automatic updates are unavailable in this build. Install a newer QuiverDL release to update.</p>}
              </div>
            </section>
          </div>
        )}
      </aside>

      <main className="workspace">
        <header
          data-tauri-drag-region
          onDoubleClick={(event) => {
            if ((event.target as HTMLElement).closest(".window-controls")) return;
            void getCurrentWindow().toggleMaximize();
          }}
        >
          <div data-tauri-drag-region>
            <p className="eyebrow" data-tauri-drag-region>DOWNLOAD MANAGER</p>
            <h1 data-tauri-drag-region>{filter === "all" ? t("downloads") : FILTER_LABELS[filter]}</h1>
          </div>
          <div className="header-actions">
            <span className="engine-badge"><i /> Engine ready</span>

          </div>
        </header>
        <div className="action-toolbar" role="toolbar" aria-label="Download actions">
          <button
            className={!settingsOpen && filter !== "completed" && sourceMode === "auto" ? "toolbar-primary" : undefined}
            type="button"
            aria-pressed={!settingsOpen && filter !== "completed" && sourceMode === "auto"}
            onClick={() => activateSourceMode("auto")}
          >
            + Add URL
          </button>
          <button
            className={!settingsOpen && filter !== "completed" && sourceMode === "media" ? "toolbar-primary" : undefined}
            type="button"
            aria-pressed={!settingsOpen && filter !== "completed" && sourceMode === "media"}
            onClick={() => activateSourceMode("media")}
          >
            Media
          </button>
          <button
            className={!settingsOpen && filter !== "completed" && sourceMode === "torrent" ? "toolbar-primary" : undefined}
            type="button"
            aria-pressed={!settingsOpen && filter !== "completed" && sourceMode === "torrent"}
            onClick={() => activateSourceMode("torrent")}
          >
            Torrent / Magnet
          </button>
          <button
            className={settings.clipboardMonitoring ? "toolbar-toggle-active" : undefined}
            type="button"
            aria-pressed={settings.clipboardMonitoring}
            onClick={() => setSettings((current) => ({
              ...current,
              clipboardMonitoring: !current.clipboardMonitoring,
            }))}
          >
            Clipboard {settings.clipboardMonitoring ? "on" : "off"}
          </button>
          <button
            className={!settingsOpen && filter === "completed" ? "toolbar-primary" : undefined}
            type="button"
            aria-pressed={!settingsOpen && filter === "completed"}
            onClick={() => {
              setFilter("completed");
              setError("");
            }}
          >
            History
          </button>
          <button
            className={settingsOpen ? "toolbar-primary" : undefined}
            type="button"
            aria-pressed={settingsOpen}
            onClick={(event) => openSettings(event.currentTarget)}
          >
            Settings
          </button>
        </div>

        {UPDATER_ENABLED && availableUpdateVersion && (
          <section className="update-banner" aria-live="polite">
            <div>
              <strong>{t("updateAvailable")} {availableUpdateVersion}</strong>
              <span>
                {updateDownloaded
                  ? t("updateReady")
                  : updateProgress === null
                    ? t("signedUpdateHint")
                    : `${t("downloadingUpdate")} ${updateProgress}%`}
              </span>
            </div>
            <button
              type="button"
              disabled={updateBusy || updateChecking}
              onClick={() => void (updateDownloaded
                ? installDownloadedUpdate()
                : downloadAvailableUpdate())}
            >
              {updateDownloaded ? t("installRestart") : t("downloadUpdate")}
            </button>
          </section>
        )}

        <section className="quick-add" aria-labelledby="quick-add-title">
          <div>
            <p className="eyebrow">{t("newDownload")}</p>
            <h2 id="quick-add-title">{sourceMode === "torrent" ? "Open a torrent or paste a magnet link" : sourceMode === "media" ? "Add a video or audio link" : t("pasteLink")}</h2>
          </div>
          <form onSubmit={inspectLink}>
            <label htmlFor="download-url">{t("downloadUrl")}</label>
            <div className="url-row">
              <input
                id="download-url"
                ref={urlInputRef}
                type="text"
                inputMode="url"
                value={url}
                onChange={(event) => {
                  setUrl(event.currentTarget.value);
                  setInspection(null);
                  setMediaInspection(null);
                  setTorrentInspection(null);
                  setReviewingBrowserRequest(null);
                }}
                placeholder={sourceMode === "media" ? "Paste a video or media page URL" : sourceMode === "torrent" ? "Paste a magnet link or torrent URL" : "https://example.com/archive.zip"}
                autoComplete="off"
                disabled={inspecting}
                required
              />
              <button className="primary" type="submit" disabled={inspecting || !url.trim()}>
                {inspecting ? t("inspecting") : t("inspect")}
              </button>
              {sourceMode === "torrent" && <button className="open-torrent-button" type="button" onClick={() => void openTorrentFile()} disabled={inspecting}>Open .torrent</button>}
            </div>
          </form>

          {error && <p className="result error" role="alert">{error}</p>}
          {updateError && <p className="result error" role="alert">{updateError}</p>}
          {inspection && (
            <div className="inspection-card">
              <div className="inspection-details">
                <div className="inspection-grid">
                  <div><span>File size</span><strong>{formatBytes(inspection.totalBytes === null ? null : BigInt(inspection.totalBytes))}</strong></div>
                  <div><span>Resume support</span><strong>{inspection.supportsRanges ? "Available" : "Unavailable"}</strong></div>
                  <div><span>Change validator</span><strong>{inspection.hasValidator ? "Protected" : "Not provided"}</strong></div>
                </div>
                <label className="schedule-field" htmlFor="scheduled-start">
                  Start later (optional)
                  <input
                    id="scheduled-start"
                    type="datetime-local"
                    step={60}
                    max="3000-01-01T00:00"
                    value={scheduledStart}
                    onChange={(event) => setScheduledStart(event.currentTarget.value)}
                  />
                  <small>Uses your local time. Leave blank to queue immediately.</small>
                </label>
              </div>
              <button className="primary save-button" type="button" onClick={chooseDestination} disabled={choosingDestination}>
                {choosingDestination ? t("opening") : t("choose")}
              </button>
            </div>
          )}
          {mediaInspection && (
            <div className="inspection-card media-inspection">
              {mediaInspection.thumbnail && (
                <img src={mediaInspection.thumbnail} alt="" referrerPolicy="no-referrer" />
              )}
              <div className="inspection-details">
                <span className="source-kind">Media via {mediaInspection.extractor}</span>
                <h3>{mediaInspection.title}</h3>
                <p>
                  {mediaInspection.durationSeconds === null
                    ? "Duration unavailable"
                    : `${Math.floor(mediaInspection.durationSeconds / 60)}:${String(mediaInspection.durationSeconds % 60).padStart(2, "0")}`}
                </p>
                <label className="media-quality">
                  Quality and format
                  <select value={mediaQuality} onChange={(event) => setMediaQuality(event.currentTarget.value)}>
                    <option value="best">Best available video</option>
                    <option value="2160">Up to 2160p</option>
                    <option value="1440">Up to 1440p</option>
                    <option value="1080">Up to 1080p</option>
                    <option value="720">Up to 720p</option>
                    <option value="480">Up to 480p</option>
                    <option value="360">Up to 360p</option>
                    <option value="audio-mp3">Audio only - MP3</option>
                    <option value="audio-m4a">Audio only - M4A</option>
                    {mediaInspection.formats.map((format) => (
                      <option key={`${format.formatId}-${format.extension}`} value={`format:${format.formatId}`}>
                        {format.label} ({format.extension}){format.approxBytes ? ` - ${formatBytes(BigInt(format.approxBytes))}` : ""}
                      </option>
                    ))}
                  </select>
                  {mediaQuality.startsWith("audio-") && <small>Audio conversion requires FFmpeg.</small>}
                </label>
              </div>
              <button className="primary save-button" type="button" onClick={chooseMediaDestination} disabled={choosingDestination}>
                {choosingDestination ? t("opening") : "Choose folder and download"}
              </button>
            </div>
          )}
          {torrentInspection && (
            <div className="inspection-card torrent-inspection" onContextMenu={(event) => { event.preventDefault(); setTorrentMenu({ x: Math.min(event.clientX, window.innerWidth - 240), y: Math.min(event.clientY, window.innerHeight - 240) }); }}>
              <div className="torrent-mark" aria-hidden="true">P2P</div>
              <div className="inspection-details">
                <span className="source-kind">{torrentInspection.sourceType === "magnet" ? "Magnet link" : "Torrent file"}</span>
                <h3>{torrentInspection.name}</h3>
                <p>Your IP address and torrent identifier can be visible to trackers and peers.</p>
                <small>Piece hashes detect corruption; they do not authenticate the publisher.</small>
                {torrentInspection.networkOrigins.length > 0 && (
                  <small>Known network origins: {torrentInspection.networkOrigins.join(", ")}</small>
                )}
                {torrentInspection.sourceType === "torrentFile" && (
                  <div className="torrent-file-picker">
                    <div className="torrent-file-toolbar">
                      <strong>{selectedTorrentFiles.size} / {torrentInspection.files?.length} files · {formatBytes((torrentInspection.files || []).reduce((sum, file, index) => sum + (selectedTorrentFiles.has(index) ? BigInt(file.bytes) : 0n), 0n))} selected</strong>
                      <button type="button" onClick={() => setSelectedTorrentFiles(new Set(torrentInspection.files?.map((_, index) => index)))}>Select all</button>
                      <button type="button" onClick={() => setSelectedTorrentFiles(new Set())}>Select none</button>
                    </div>
                    <input type="search" aria-label="Find torrent files" placeholder="Find a file or folder…" value={torrentFileSearch} onChange={(event) => setTorrentFileSearch(event.target.value)} />
                    <div className="torrent-file-list" role="group" aria-label="Files to download">
                      {torrentInspection.files?.map((file, index) => ({ file, index })).filter(({ file }) => file.path.toLowerCase().includes(torrentFileSearch.toLowerCase())).map(({ file, index }) => (
                        <label key={file.path} onContextMenu={(event) => { event.preventDefault(); event.stopPropagation(); setTorrentMenu({ index, x: Math.min(event.clientX, window.innerWidth - 240), y: Math.min(event.clientY, window.innerHeight - 240) }); }}>
                          <input type="checkbox" checked={selectedTorrentFiles.has(index)} onChange={(event) => setSelectedTorrentFiles((current) => { const next = new Set(current); if (event.target.checked) next.add(index); else next.delete(index); return next; })} />
                          <span>{file.path}</span><small>{formatBytes(BigInt(file.bytes))}</small>
                        </label>
                      ))}
                    </div>
                  </div>
                )}
                <label>Download folder<div className="path-picker-row"><input readOnly value={torrentDestination} placeholder="Choose a folder" /><button type="button" onClick={() => void browseTorrentDestination()}>Browse…</button></div></label>
                <label className="torrent-consent">
                  <input
                    type="checkbox"
                    checked={torrentPrivacyConfirmed}
                    onChange={(event) => setTorrentPrivacyConfirmed(event.currentTarget.checked)}
                  />
                  <span>
                    I understand that tracker contact, outbound TCP peer connections, and peer exchange may reveal this download. QuiverDL disables DHT, local discovery, incoming listeners, and uploading. Pause may retain tracker or peer state; cancel or completion stops this torrent session.
                  </span>
                </label>
                {settings.proxyMode !== "disabled" && (
                  <small className="torrent-proxy-warning">
                    Torrent startup is blocked while System or Custom HTTP proxy mode is active because it cannot cover every peer transport.
                  </small>
                )}
              </div>
              <div className="torrent-start-actions">
                {torrentInspection.sourceType === "magnet" ? <button className="primary" type="button" onClick={() => void loadMagnetFileList()} disabled={torrentMetadataBusy || !torrentPrivacyConfirmed || settings.proxyMode !== "disabled"}>{torrentMetadataBusy ? "Retrieving file list…" : "Preview files"}</button> : <button className="primary" type="button" onClick={chooseTorrentDestination} disabled={choosingDestination || !torrentPrivacyConfirmed || settings.proxyMode !== "disabled" || selectedTorrentFiles.size === 0}>{choosingDestination ? t("opening") : "Download selected files"}</button>}
                <button type="button" disabled={torrentMetadataBusy} onClick={() => { setTorrentInspection(null); setReviewingBrowserRequest(null); }}>Close preview</button>
              </div>
            </div>
          )}
          {torrentMenu && <div className="torrent-context-menu" role="menu" aria-label="Torrent options" style={{ left: torrentMenu.x, top: torrentMenu.y }}>
            <button role="menuitem" type="button" autoFocus onClick={() => void browseTorrentDestination()}>Choose download folder…</button>
            {torrentMenu.index !== undefined && <button role="menuitem" type="button" onClick={() => { setSelectedTorrentFiles(new Set([torrentMenu.index!])); setTorrentMenu(null); }}>Download only this file</button>}
            {torrentMenu.index !== undefined && <button role="menuitem" type="button" onClick={() => { setSelectedTorrentFiles((current) => { const next = new Set(current); next.delete(torrentMenu.index!); return next; }); setTorrentMenu(null); }}>Skip this file</button>}
            <button role="menuitem" type="button" onClick={() => { setSelectedTorrentFiles(new Set(torrentInspection?.files?.map((_, index) => index))); setTorrentMenu(null); }}>Select all files</button>
            <button role="menuitem" type="button" onClick={() => { setSelectedTorrentFiles(new Set()); setTorrentMenu(null); }}>Select no files</button>
          </div>}
        </section>

        {browserRequests.length > 0 && (
          <section className="browser-inbox" aria-label="Browser download requests">
            <div>
              <strong>{browserRequests.length} browser {browserRequests.length === 1 ? "request" : "requests"}</strong>
              <span>Review before choosing a save location. QuiverDL never captures cookies or browsing history.</span>
            </div>
            {browserRequests.slice(0, 3).map((request) => (
              <button type="button" key={request.id} onClick={() => void reviewBrowserRequest(request)}>
                Review {request.suggestedFilename ?? (() => { try { return new URL(request.url).hostname; } catch { return "download"; } })()}
              </button>
            ))}
          </section>
        )}

        {clipboardCandidate && (
          <aside className="clipboard-toast" role="status" aria-live="polite">
            <div>
              <strong>Download link copied</strong>
              <span>{clipboardCandidate.kind === "media" ? "Media" : clipboardCandidate.kind === "torrent" ? "Torrent" : "File"} link detected</span>
            </div>
            <button className="primary" type="button" onClick={acceptClipboardCandidate}>Add</button>
            <button type="button" onClick={() => setClipboardCandidate(null)}>Dismiss</button>
          </aside>
        )}

        <section className="downloads-panel" aria-live="polite">
          <div className="panel-heading">
            <h2>{filter === "all"
              ? "All downloads"
              : filter === "completed" ? t("downloadHistory") : FILTER_LABELS[filter]}</h2>
            <span>
              {filter === "completed" && historyQuery.trim()
                ? `${visibleDownloads.length} ${t("of")} ${counts.completed}`
                : visibleDownloads.length}{" "}
              {visibleDownloads.length === 1 ? t("item") : t("items")}
            </span>
          </div>
          {filter === "completed" && (
            <div className="history-toolbar" role="search">
              <label>
                <span className="sr-only">{t("searchHistory")}</span>
                <input
                  type="search"
                  value={historyQuery}
                  placeholder={t("searchHistory")}
                  onChange={(event) => setHistoryQuery(event.target.value)}
                />
              </label>
              <label>
                <span className="sr-only">{t("sortHistory")}</span>
                <select
                  aria-label={t("sortHistory")}
                  value={historySort}
                  onChange={(event) => setHistorySort(event.target.value as HistorySort)}
                >
                  <option value="newest">{t("newestFirst")}</option>
                  <option value="oldest">{t("oldestFirst")}</option>
                  <option value="name">{t("nameSort")}</option>
                  <option value="size">{t("sizeSort")}</option>
                </select>
              </label>
              <button
                className="clear-history"
                type="button"
                disabled={counts.completed === 0}
                onClick={clearCompletedHistory}
              >
                {t("clearCompleted")}
              </button>
            </div>
          )}
          {visibleDownloads.length === 0 ? (
            <div className="empty-state">
              <div className="target-icon" aria-hidden="true"><img src={quiverLogo} alt="" draggable={false} /></div>
              <h3>{filter === "completed" ? t("noHistory") : downloads.length === 0 ? t("empty") : "Nothing in this view"}</h3>
              <p>
                {filter === "completed"
                  ? historyQuery.trim() ? t("noHistoryMatch") : t("noHistoryHint")
                  : downloads.length === 0
                  ? t("emptyHint")
                  : "Choose another filter to see your downloads."}
              </p>
            </div>
          ) : (
            <div className="download-list">
              {visibleDownloads.map((item) => (
                <DownloadRow
                  item={item}
                  key={item.id}
                  onControl={(action) => void controlDownload(item, action)}
                  onRemove={() => removeDownload(item.id)}
                  onRetry={() => retryDownload(item)}
                  completedLabel={item.status === "completed"
                    ? item.completedAtMs === null
                      ? t("completionDateUnavailable")
                      : `${t("completedOn")} ${formatHistoryTime(item.completedAtMs, settings.language)}`
                    : null}
                  removeLabel={item.status === "completed" ? t("removeFromHistory") : t("remove")}
                />
              ))}
            </div>
          )}
        </section>
      </main>
    </div>
  );
}

async function notifyCompleted(name: string) {
  let granted = await isPermissionGranted();
  if (!granted) {
    granted = (await requestPermission()) === "granted";
  }
  if (granted) {
    sendNotification({ title: "QuiverDL", body: `${name} finished downloading.` });
  }
}

function FilterButton({ active, count, label, onClick }: { active: boolean; count: number; label: string; onClick: () => void }) {
  return (
    <button className={`nav-item${active ? " active" : ""}`} type="button" aria-pressed={active} onClick={onClick}>
      <span className="nav-dot" aria-hidden="true" />
      {label}
      <b>{count}</b>
    </button>
  );
}

function DownloadRow({ item, onControl, onRemove, onRetry, completedLabel, removeLabel }: { item: DownloadItem; onControl: (action: "pause" | "resume" | "cancel") => void; onRemove: () => void; onRetry: () => void; completedLabel: string | null; removeLabel: string }) {
  const percentage = item.totalBytes !== null && item.totalBytes > 0n
    ? Math.min(100, Number((item.downloadedBytes * 1000n) / item.totalBytes) / 10)
    : null;
  const isActive = ACTIVE_STATUSES.has(item.status);
  const canPause = item.kind !== "media" && ["probing", "downloading"].includes(item.status);
  const canCancel = isActive && item.status !== "cancelling";
  const hostname = (() => {
    try { return new URL(item.url).hostname; } catch { return "download"; }
  })();

  return (
    <article className={`download-row status-${item.status}`}>
      <div className={`file-badge kind-${item.kind}`} aria-hidden="true">
        {item.kind === "media" ? "MEDIA" : item.kind === "torrent" ? "P2P" : "FILE"}
      </div>
      <div className="download-content">
        <div className="download-title">
          <div>
            <h3 title={item.destination}>{item.name}</h3>
            <p>{hostname}</p>
          </div>
          <span className="status-pill">{STATUS_LABELS[item.status]}</span>
        </div>
        <progress max={100} value={percentage ?? undefined} aria-label={`Download progress for ${item.name}`} />
        <div className="download-meta">
          <span>{formatBytes(item.downloadedBytes)}{item.totalBytes !== null ? ` of ${formatBytes(item.totalBytes)}` : ""}</span>
          <span>{completedLabel ?? (percentage === null ? "Size unknown" : `${percentage.toFixed(0)}%`)}</span>
        </div>
        {item.status === "scheduled" && item.scheduledForMs && (
          <p className="queue-note">Starts {formatScheduledTime(item.scheduledForMs)}</p>
        )}
        {item.status === "queued" && <p className="queue-note">Waiting for the previous queued download</p>}
        {item.error && <p className="item-error" role="alert">{item.error}</p>}
        {item.status === "completed" && item.sha256 && (
          <p className="checksum" title={item.sha256}>SHA-256 {item.sha256.slice(0, 16)}...{item.resumed ? " (resumed)" : ""}</p>
        )}
      </div>
      <div className="row-actions">
        {canPause && <button type="button" onClick={() => onControl("pause")}>Pause</button>}
        {item.status === "paused" && <button type="button" onClick={() => item.recoverable ? onRetry() : onControl("resume")}>Resume</button>}
        {canCancel && <button className="danger" type="button" onClick={() => onControl("cancel")}>Cancel</button>}
        {(item.status === "failed" || item.status === "cancelled") && <button type="button" onClick={onRetry}>Retry</button>}
        {!isActive && <button type="button" onClick={onRemove}>{removeLabel}</button>}
      </div>
    </article>
  );
}

export default App;
