import { invoke } from "@tauri-apps/api/core";
import { FormEvent, useState } from "react";
import "./App.css";

type LinkInspection = {
  effectiveUrl: string;
  totalBytes: number | null;
  supportsRanges: boolean;
  hasValidator: boolean;
};

function formatBytes(bytes: number | null) {
  if (bytes === null) return "Unknown size";
  const units = ["B", "KB", "MB", "GB", "TB"];
  let value = bytes;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return `${value.toFixed(unit === 0 ? 0 : 1)} ${units[unit]}`;
}

function App() {
  const [url, setUrl] = useState("");
  const [inspection, setInspection] = useState<LinkInspection | null>(null);
  const [error, setError] = useState("");
  const [loading, setLoading] = useState(false);

  async function inspectLink(event: FormEvent) {
    event.preventDefault();
    if (!url.trim()) return;
    setLoading(true);
    setError("");
    setInspection(null);
    try {
      setInspection(await invoke<LinkInspection>("inspect_url", { url }));
    } catch (cause) {
      setError(String(cause));
    } finally {
      setLoading(false);
    }
  }

  return (
    <div className="app-shell">
      <aside className="sidebar">
        <div className="brand">
          <span className="brand-mark" aria-hidden="true">↓</span>
          <span>QuiverDL</span>
        </div>
        <nav aria-label="Download filters">
          <button className="nav-item active"><span>⌁</span> Downloads</button>
          <button className="nav-item"><span>↯</span> Active <b>0</b></button>
          <button className="nav-item"><span>≡</span> Queued <b>0</b></button>
          <button className="nav-item"><span>✓</span> Completed <b>0</b></button>
        </nav>
        <button className="nav-item settings"><span>⚙</span> Settings</button>
        <div className="privacy-note">Private by design<br /><small>No accounts. No telemetry.</small></div>
      </aside>

      <main className="workspace">
        <header>
          <div>
            <p className="eyebrow">DOWNLOAD MANAGER</p>
            <h1>Downloads</h1>
          </div>
          <span className="engine-badge"><i /> Engine ready</span>
        </header>

        <section className="quick-add" aria-labelledby="quick-add-title">
          <div>
            <p className="eyebrow">QUICK ADD</p>
            <h2 id="quick-add-title">Aim a link at QuiverDL</h2>
          </div>
          <form onSubmit={inspectLink}>
            <label htmlFor="download-url">Download URL</label>
            <div className="url-row">
              <input
                id="download-url"
                type="url"
                value={url}
                onChange={(event) => setUrl(event.currentTarget.value)}
                placeholder="https://example.com/large-file.zip"
                autoComplete="off"
              />
              <button className="primary" type="submit" disabled={loading || !url.trim()}>
                {loading ? "Inspecting…" : "Inspect link"}
              </button>
            </div>
          </form>

          {error && <p className="result error" role="alert">{error}</p>}
          {inspection && (
            <div className="result success">
              <div><span>File size</span><strong>{formatBytes(inspection.totalBytes)}</strong></div>
              <div><span>Resume support</span><strong>{inspection.supportsRanges ? "Available" : "Unavailable"}</strong></div>
              <div><span>Change validator</span><strong>{inspection.hasValidator ? "Protected" : "Not provided"}</strong></div>
            </div>
          )}
        </section>

        <section className="downloads-panel">
          <div className="panel-heading">
            <h2>All downloads</h2>
            <span>0 items</span>
          </div>
          <div className="empty-state">
            <div className="target-icon" aria-hidden="true"><span>↓</span></div>
            <h3>Your quiver is empty</h3>
            <p>Paste a direct HTTP or HTTPS link above to inspect your first download.</p>
          </div>
        </section>
      </main>
    </div>
  );
}

export default App;
