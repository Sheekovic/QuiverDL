import { Channel, invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { save } from "@tauri-apps/plugin-dialog";
import {
  isPermissionGranted,
  requestPermission,
  sendNotification,
} from "@tauri-apps/plugin-notification";
import { FormEvent, useEffect, useMemo, useRef, useState } from "react";
import "./App.css";
import { MessageKey, translate } from "./i18n";

type LinkInspectionResponse = {
  effectiveUrl: string;
  totalBytes: string | null;
  supportsRanges: boolean;
  hasValidator: boolean;
  suggestedFilename: string;
};

type LinkInspection = LinkInspectionResponse & {
  sourceUrl: string;
};

type EngineStatus = "probing" | "retrying" | "downloading" | "verifying" | "completed";
type DownloadStatus =
  | EngineStatus
  | "starting"
  | "paused"
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
};

type AppSettings = {
  theme: "system" | "light" | "dark";
  language: "en" | "ar";
  notifications: boolean;
  retryAttempts: number;
  retryInitialDelayMs: number;
  retryMaxDelayMs: number;
  maxSegments: number;
  maxConnectionsPerHost: number;
  perDownloadSpeedLimitBps: number | null;
  globalSpeedLimitBps: number | null;
  proxyMode: "disabled" | "system" | "custom";
  proxyUrl: string;
  proxyUsername: string;
  proxyBypass: string;
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
};

type BrowserBridgeInfo = {
  hostName: string;
  token: string;
  configPath: string;
};

type Filter = "all" | "active" | "completed" | "failed";

const ACTIVE_STATUSES = new Set<DownloadStatus>([
  "starting",
  "probing",
  "retrying",
  "downloading",
  "paused",
  "verifying",
  "cancelling",
]);

const STATUS_LABELS: Record<DownloadStatus, string> = {
  starting: "Starting",
  probing: "Inspecting server",
  retrying: "Retrying",
  downloading: "Downloading",
  paused: "Paused",
  verifying: "Verifying",
  cancelling: "Cancelling",
  completed: "Completed",
  cancelled: "Cancelled",
  failed: "Failed",
};

const DEFAULT_SETTINGS: AppSettings = {
  theme: "system",
  language: "en",
  notifications: true,
  retryAttempts: 3,
  retryInitialDelayMs: 750,
  retryMaxDelayMs: 15_000,
  maxSegments: 4,
  maxConnectionsPerHost: 8,
  perDownloadSpeedLimitBps: null,
  globalSpeedLimitBps: null,
  proxyMode: "disabled",
  proxyUrl: "",
  proxyUsername: "",
  proxyBypass: "",
};

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

function App() {
  const [url, setUrl] = useState("");
  const [inspection, setInspection] = useState<LinkInspection | null>(null);
  const [downloads, setDownloads] = useState<DownloadItem[]>([]);
  const [filter, setFilter] = useState<Filter>("all");
  const [error, setError] = useState("");
  const [inspecting, setInspecting] = useState(false);
  const [choosingDestination, setChoosingDestination] = useState(false);
  const [settings, setSettings] = useState<AppSettings>(DEFAULT_SETTINGS);
  const [browserRequests, setBrowserRequests] = useState<BrowserRequest[]>([]);
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
  const saveTimer = useRef<number | null>(null);
  const saveInFlight = useRef(false);
  const saveAgain = useRef(false);
  const latestSnapshot = useRef<AppSnapshot | null>(null);
  const quitInProgress = useRef(false);
  const t = (key: MessageKey) => translate(settings.language, key);

  useEffect(() => {
    let active = true;
    void invoke<AppSnapshot>("load_app_state")
      .then((snapshot) => {
        if (!active) return;
        setSettings(snapshot.settings);
        setProxyDraft(proxyDraftFromSettings(snapshot.settings));
        setDownloads(
          snapshot.downloads.map((item) => ({
            ...item,
            downloadedBytes: BigInt(item.downloadedBytes),
            totalBytes: item.totalBytes === null ? null : BigInt(item.totalBytes),
            recoverable: item.status === "paused",
          })),
        );
        stateLoaded.current = true;
      })
      .catch((cause) => {
        if (active) setError(String(cause));
        stateLoaded.current = true;
      });
    return () => {
      active = false;
    };
  }, []);

  useEffect(() => {
    let active = true;
    const refresh = () => {
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
    latestSnapshot.current = {
      schemaVersion: 1,
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
  }, [settings.language, settings.theme]);

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
  }, [proxyDraft.proxyMode, proxyDraft.proxyUsername]);

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

  const visibleDownloads = useMemo(
    () =>
      downloads.filter((item) => {
        if (filter === "active") return ACTIVE_STATUSES.has(item.status);
        if (filter === "completed") return item.status === "completed";
        if (filter === "failed") {
          return item.status === "failed" || item.status === "cancelled";
        }
        return true;
      }),
    [downloads, filter],
  );

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
    try {
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
    setChoosingDestination(true);
    setError("");
    try {
      const destination = await save({
        title: "Save download as",
        defaultPath: inspection.suggestedFilename || filenameFromUrl(inspection.effectiveUrl),
      });
      if (!destination) return;

      const sourceUrl = inspection.sourceUrl;
      const browserRequestId =
        reviewingBrowserRequest?.url === sourceUrl ? reviewingBrowserRequest.id : undefined;
      setUrl("");
      setInspection(null);
      void runDownload(sourceUrl, destination, undefined, browserRequestId);
    } catch (cause) {
      setError(String(cause));
    } finally {
      setChoosingDestination(false);
    }
  }

  async function runDownload(
    sourceUrl: string,
    destination: string,
    existingId?: string,
    browserRequestId?: string,
  ) {
    const id = existingId ?? createTaskId();
    const item: DownloadItem = {
      id,
      name: destinationName(destination),
      url: sourceUrl,
      destination,
      status: "starting",
      downloadedBytes: 0n,
      totalBytes: null,
      recoverable: false,
    };
    setDownloads((current) =>
      existingId
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
      schemaVersion: 1,
      settings,
      downloads: downloads.map(({ recoverable: _recoverable, ...download }) => ({
        ...download,
        downloadedBytes: download.downloadedBytes.toString(),
        totalBytes: download.totalBytes?.toString() ?? null,
      })),
    };
    const snapshot: AppSnapshot = {
      schemaVersion: 1,
      settings,
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

    const onEvent = new Channel<DownloadProgress>();
    onEvent.onmessage = (message) => {
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
        settings,
        onEvent,
      });
      updateDownload(id, {
        status: "completed",
        downloadedBytes: BigInt(summary.bytesWritten),
        totalBytes: BigInt(summary.bytesWritten),
        sha256: summary.sha256,
        resumed: summary.resumed,
      });
      if (settings.notifications) {
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
    }
  }

  function retryDownload(item: DownloadItem) {
    void runDownload(item.url, item.destination, item.id);
  }

  function reviewBrowserRequest(request: BrowserRequest) {
    setUrl(request.url);
    setInspection(null);
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

  return (
    <div className="app-shell">
      <aside className="sidebar">
        <div className="brand">
          <span className="brand-mark" aria-hidden="true">DL</span>
          <span>QuiverDL</span>
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
        <details className="settings-panel">
          <summary>{t("settings")}</summary>
          <label>
            {t("theme")}
            <select value={settings.theme} onChange={(event) => setSettings((current) => ({ ...current, theme: event.target.value as AppSettings["theme"] }))}>
              <option value="system">System</option>
              <option value="light">Light</option>
              <option value="dark">Dark</option>
            </select>
          </label>
          <label>
            {t("language")}
            <select value={settings.language} onChange={(event) => setSettings((current) => ({ ...current, language: event.target.value as AppSettings["language"] }))}>
              <option value="en">English</option>
              <option value="ar">العربية</option>
            </select>
          </label>
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
          <label className="checkbox-setting">
            <input type="checkbox" checked={settings.notifications} onChange={(event) => setSettings((current) => ({ ...current, notifications: event.target.checked }))} />
            {t("notifications")}
          </label>
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
                    onChange={(event) =>
                      setProxyDraft((current) => ({ ...current, proxyUrl: event.target.value }))
                    }
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
          <button className="bridge-button" type="button" onClick={() => void revealBrowserBridge()}>
            Browser extension setup
          </button>
          {bridgeInfo && (
            <div className="bridge-secret">
              <span>Native host</span>
              <code>{bridgeInfo.hostName}</code>
              <span>Pairing token</span>
              <code>{bridgeInfo.token}</code>
              <small title={bridgeInfo.configPath}>Keep this token private.</small>
            </div>
          )}
        </details>
      </aside>

      <main className="workspace">
        <header>
          <div>
            <p className="eyebrow">DOWNLOAD MANAGER</p>
            <h1>{filter === "all" ? t("downloads") : FILTER_LABELS[filter]}</h1>
          </div>
          <span className="engine-badge"><i /> Engine ready</span>
        </header>

        <section className="quick-add" aria-labelledby="quick-add-title">
          <div>
            <p className="eyebrow">{t("newDownload")}</p>
            <h2 id="quick-add-title">{t("pasteLink")}</h2>
          </div>
          <form onSubmit={inspectLink}>
            <label htmlFor="download-url">{t("downloadUrl")}</label>
            <div className="url-row">
              <input
                id="download-url"
                type="url"
                value={url}
                onChange={(event) => {
                  setUrl(event.currentTarget.value);
                  setInspection(null);
                  setReviewingBrowserRequest(null);
                }}
                placeholder="https://example.com/archive.zip"
                autoComplete="off"
                disabled={inspecting}
                required
              />
              <button className="primary" type="submit" disabled={inspecting || !url.trim()}>
                {inspecting ? t("inspecting") : t("inspect")}
              </button>
            </div>
          </form>

          {error && <p className="result error" role="alert">{error}</p>}
          {inspection && (
            <div className="inspection-card">
              <div className="inspection-grid">
                <div><span>File size</span><strong>{formatBytes(inspection.totalBytes === null ? null : BigInt(inspection.totalBytes))}</strong></div>
                <div><span>Resume support</span><strong>{inspection.supportsRanges ? "Available" : "Unavailable"}</strong></div>
                <div><span>Change validator</span><strong>{inspection.hasValidator ? "Protected" : "Not provided"}</strong></div>
              </div>
              <button className="primary save-button" type="button" onClick={chooseDestination} disabled={choosingDestination}>
                {choosingDestination ? t("opening") : t("choose")}
              </button>
            </div>
          )}
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

        <section className="downloads-panel" aria-live="polite">
          <div className="panel-heading">
            <h2>{filter === "all" ? "All downloads" : FILTER_LABELS[filter]}</h2>
            <span>{visibleDownloads.length} {visibleDownloads.length === 1 ? "item" : "items"}</span>
          </div>
          {visibleDownloads.length === 0 ? (
            <div className="empty-state">
              <div className="target-icon" aria-hidden="true"><span>DL</span></div>
              <h3>{downloads.length === 0 ? t("empty") : "Nothing in this view"}</h3>
              <p>
                {downloads.length === 0
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

function DownloadRow({ item, onControl, onRemove, onRetry }: { item: DownloadItem; onControl: (action: "pause" | "resume" | "cancel") => void; onRemove: () => void; onRetry: () => void }) {
  const percentage = item.totalBytes !== null && item.totalBytes > 0n
    ? Math.min(100, Number((item.downloadedBytes * 1000n) / item.totalBytes) / 10)
    : null;
  const isActive = ACTIVE_STATUSES.has(item.status);
  const canPause = ["probing", "downloading"].includes(item.status);
  const canCancel = isActive && item.status !== "cancelling";
  const hostname = (() => {
    try { return new URL(item.url).hostname; } catch { return "download"; }
  })();

  return (
    <article className={`download-row status-${item.status}`}>
      <div className="file-badge" aria-hidden="true">FILE</div>
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
          <span>{percentage === null ? "Size unknown" : `${percentage.toFixed(0)}%`}</span>
        </div>
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
        {!isActive && <button type="button" onClick={onRemove}>Remove</button>}
      </div>
    </article>
  );
}

export default App;
