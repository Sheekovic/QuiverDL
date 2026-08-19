import { Channel, invoke } from "@tauri-apps/api/core";
import { save } from "@tauri-apps/plugin-dialog";
import { FormEvent, useMemo, useState } from "react";
import "./App.css";

type LinkInspection = {
  effectiveUrl: string;
  totalBytes: string | null;
  supportsRanges: boolean;
  hasValidator: boolean;
};

type EngineStatus = "probing" | "downloading" | "verifying" | "completed";
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
};

type Filter = "all" | "active" | "completed" | "failed";

const ACTIVE_STATUSES = new Set<DownloadStatus>([
  "starting",
  "probing",
  "downloading",
  "paused",
  "verifying",
  "cancelling",
]);

const STATUS_LABELS: Record<DownloadStatus, string> = {
  starting: "Starting",
  probing: "Inspecting server",
  downloading: "Downloading",
  paused: "Paused",
  verifying: "Verifying",
  cancelling: "Cancelling",
  completed: "Completed",
  cancelled: "Cancelled",
  failed: "Failed",
};

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

  async function inspectLink(event: FormEvent) {
    event.preventDefault();
    const submittedUrl = url.trim();
    if (!submittedUrl) return;
    setInspecting(true);
    setError("");
    setInspection(null);
    try {
      setInspection(await invoke<LinkInspection>("inspect_url", { url: submittedUrl }));
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
        defaultPath: filenameFromUrl(inspection.effectiveUrl),
      });
      if (!destination) return;

      const sourceUrl = inspection.effectiveUrl;
      setUrl("");
      setInspection(null);
      void runDownload(sourceUrl, destination);
    } catch (cause) {
      setError(String(cause));
    } finally {
      setChoosingDestination(false);
    }
  }

  async function runDownload(sourceUrl: string, destination: string) {
    const id = createTaskId();
    const item: DownloadItem = {
      id,
      name: destinationName(destination),
      url: sourceUrl,
      destination,
      status: "starting",
      downloadedBytes: 0n,
      totalBytes: null,
    };
    setDownloads((current) => [item, ...current]);
    setFilter("all");

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
        onEvent,
      });
      updateDownload(id, {
        status: "completed",
        downloadedBytes: BigInt(summary.bytesWritten),
        totalBytes: BigInt(summary.bytesWritten),
        sha256: summary.sha256,
        resumed: summary.resumed,
      });
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
          <FilterButton active={filter === "all"} count={downloads.length} label="Downloads" onClick={() => setFilter("all")} />
          <FilterButton active={filter === "active"} count={counts.active} label="Active" onClick={() => setFilter("active")} />
          <FilterButton active={filter === "completed"} count={counts.completed} label="Completed" onClick={() => setFilter("completed")} />
          <FilterButton active={filter === "failed"} count={counts.failed} label="Needs attention" onClick={() => setFilter("failed")} />
        </nav>
        <div className="privacy-note">
          Private by design
          <small>No accounts. No telemetry.</small>
        </div>
      </aside>

      <main className="workspace">
        <header>
          <div>
            <p className="eyebrow">DOWNLOAD MANAGER</p>
            <h1>{filter === "all" ? "Downloads" : FILTER_LABELS[filter]}</h1>
          </div>
          <span className="engine-badge"><i /> Engine ready</span>
        </header>

        <section className="quick-add" aria-labelledby="quick-add-title">
          <div>
            <p className="eyebrow">NEW DOWNLOAD</p>
            <h2 id="quick-add-title">Paste a direct HTTP or HTTPS link</h2>
          </div>
          <form onSubmit={inspectLink}>
            <label htmlFor="download-url">Download URL</label>
            <div className="url-row">
              <input
                id="download-url"
                type="url"
                value={url}
                onChange={(event) => {
                  setUrl(event.currentTarget.value);
                  setInspection(null);
                }}
                placeholder="https://example.com/archive.zip"
                autoComplete="off"
                disabled={inspecting}
                required
              />
              <button className="primary" type="submit" disabled={inspecting || !url.trim()}>
                {inspecting ? "Inspecting..." : "Inspect link"}
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
                {choosingDestination ? "Opening..." : "Choose location and download"}
              </button>
            </div>
          )}
        </section>

        <section className="downloads-panel" aria-live="polite">
          <div className="panel-heading">
            <h2>{filter === "all" ? "All downloads" : FILTER_LABELS[filter]}</h2>
            <span>{visibleDownloads.length} {visibleDownloads.length === 1 ? "item" : "items"}</span>
          </div>
          {visibleDownloads.length === 0 ? (
            <div className="empty-state">
              <div className="target-icon" aria-hidden="true"><span>DL</span></div>
              <h3>{downloads.length === 0 ? "Your queue is empty" : "Nothing in this view"}</h3>
              <p>
                {downloads.length === 0
                  ? "Paste a direct link above, inspect it, and choose where to save the file."
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
                />
              ))}
            </div>
          )}
        </section>
      </main>
    </div>
  );
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

function DownloadRow({ item, onControl, onRemove }: { item: DownloadItem; onControl: (action: "pause" | "resume" | "cancel") => void; onRemove: () => void }) {
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
        {item.status === "paused" && <button type="button" onClick={() => onControl("resume")}>Resume</button>}
        {canCancel && <button className="danger" type="button" onClick={() => onControl("cancel")}>Cancel</button>}
        {!isActive && <button type="button" onClick={onRemove}>Remove</button>}
      </div>
    </article>
  );
}

export default App;
