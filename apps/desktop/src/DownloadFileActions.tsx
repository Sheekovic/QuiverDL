import { invoke } from "@tauri-apps/api/core";
import { useRef, useState } from "react";
import { Language, translate } from "./i18n";

export function DownloadFileActions({ destination, directory, language }: {
  destination: string;
  directory: boolean;
  language: Language;
}) {
  const pending = useRef(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");

  async function act(action: "open" | "reveal") {
    if (pending.current) return;
    pending.current = true;
    setBusy(true);
    setError("");
    try {
      await invoke("open_download_destination", { destination, action });
    } catch (cause) {
      setError(`${translate(language, "fileActionFailed")} ${String(cause)}`);
    } finally {
      pending.current = false;
      setBusy(false);
    }
  }

  return (
    <div className="completed-file-actions" aria-busy={busy}>
      <button type="button" disabled={busy} onClick={() => void act("open")}>
        {translate(language, directory ? "openFolder" : "openFile")}
      </button>
      <button type="button" disabled={busy} onClick={() => void act("reveal")}>
        {translate(language, "showInFolder")}
      </button>
      {error && <p className="item-error" role="alert">{error}</p>}
    </div>
  );
}
