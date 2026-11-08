import { type ReactNode, useEffect, useId, useRef } from "react";
import { formatEta, formatRate } from "./transferMetrics";

const labels = {
  en: { details: "Download details", close: "Close", speed: "Download speed", eta: "Time remaining", size: "File size", downloaded: "Downloaded", source: "Source", destination: "Save location", graph: "Download speed · last 60 seconds", waiting: "Waiting for transfer measurements", unknown: "Unknown", complete: "Complete", paused: "Paused", estimating: "Estimating…", finalizing: "Finishing…" },
  ar: { details: "تفاصيل التنزيل", close: "إغلاق", speed: "سرعة التنزيل", eta: "الوقت المتبقي", size: "حجم الملف", downloaded: "تم تنزيل", source: "المصدر", destination: "مكان الحفظ", graph: "سرعة التنزيل · آخر 60 ثانية", waiting: "في انتظار قياسات التنزيل", unknown: "غير معروف", complete: "مكتمل", paused: "متوقف مؤقتاً", estimating: "جارٍ التقدير…", finalizing: "جارٍ الإنهاء…" },
};

export function transferLabels(language: "en" | "ar", status: string, rate: number | null, eta: number | null, totalKnown: boolean) {
  const text = labels[language];
  const downloading = status === "downloading";
  return {
    text,
    speed: downloading ? formatRate(rate) : "—",
    eta: status === "completed" ? text.complete : status === "paused" ? text.paused
      : status === "verifying" || status === "extracting" ? text.finalizing
      : downloading ? !totalKnown ? text.unknown : eta === null ? text.estimating : formatEta(eta) : "—",
  };
}

type Props = {
  open: boolean;
  onClose: () => void;
  name: string;
  language: "en" | "ar";
  statusLabel: string;
  status: string;
  source: string;
  destination: string;
  downloaded: string;
  total: string;
  totalKnown: boolean;
  percentage: number | null;
  bytesPerSecond: number | null;
  etaSeconds: number | null;
  history: (number | null)[];
  actions: ReactNode;
  error?: string;
};

export function DownloadDetails(props: Props) {
  const dialog = useRef<HTMLDialogElement>(null);
  const titleId = useId();
  const { text, speed, eta } = transferLabels(props.language, props.status, props.bytesPerSecond, props.etaSeconds, props.totalKnown);
  useEffect(() => {
    const element = dialog.current;
    if (props.open && !element?.open) element?.showModal();
    if (!props.open && element?.open) element.close();
  }, [props.open]);
  const peak = Math.max(1, ...props.history.map((rate) => rate ?? 0));
  const path = props.history.map((rate, index) => rate === null ? ""
    : `${index === 0 || props.history[index - 1] === null ? "M" : "L"}${(59 - props.history.length + 1 + index) * 10},${116 - rate / peak * 108}`).join(" ");

  return (
    <dialog ref={dialog} className="transfer-dialog" aria-labelledby={titleId} aria-live="off" dir={props.language === "ar" ? "rtl" : "ltr"} onClose={props.onClose} onCancel={props.onClose}>
      <header className="transfer-dialog-heading">
        <div><p>{text.details}</p><h2 id={titleId}>{props.name}</h2></div>
        <button type="button" className="transfer-close" onClick={props.onClose} aria-label={text.close}>×</button>
      </header>
      <div className="transfer-status"><span className={`status-pill status-${props.status}`}>{props.statusLabel}</span><strong>{props.percentage === null ? text.unknown : `${props.percentage.toFixed(1)}%`}</strong></div>
      <progress max={100} value={props.percentage ?? undefined} aria-label={text.downloaded} />
      <dl className="transfer-stats">
        <div><dt>{text.speed}</dt><dd dir="ltr">{speed}</dd></div>
        <div><dt>{text.eta}</dt><dd>{eta}</dd></div>
        <div><dt>{text.downloaded}</dt><dd dir="ltr">{props.downloaded}</dd></div>
        <div><dt>{text.size}</dt><dd dir="ltr">{props.total}</dd></div>
      </dl>
      <figure className="transfer-graph">
        <figcaption><span>{text.graph}</span><span dir="ltr">{formatRate(props.history.some((rate) => rate !== null) ? peak : null)}</span></figcaption>
        <svg viewBox="0 0 590 124" preserveAspectRatio="none" aria-hidden="true">
          <path className="graph-grid" d="M0 8H590 M0 44H590 M0 80H590 M0 116H590" />
          <path className="graph-line" d={path} />
        </svg>
        {!props.history.some((rate) => rate !== null) && <p>{text.waiting}</p>}
      </figure>
      <dl className="transfer-paths">
        <div><dt>{text.source}</dt><dd dir="ltr">{props.source}</dd></div>
        <div><dt>{text.destination}</dt><dd dir="ltr">{props.destination}</dd></div>
      </dl>
      {props.error && <p className="item-error" role="alert">{props.error}</p>}
      <footer className="row-actions transfer-actions">{props.actions}<button type="button" onClick={props.onClose}>{text.close}</button></footer>
    </dialog>
  );
}
