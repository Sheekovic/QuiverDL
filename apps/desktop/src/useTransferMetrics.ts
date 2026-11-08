import { useEffect, useRef, useState } from "react";
import { TransferMeter, type TransferMetrics } from "./transferMetrics";

export function useTransferMetrics(bytes: bigint, total: bigint | null, status: string) {
  const meter = useRef(new TransferMeter());
  const latest = useRef({ bytes, total, status });
  const [metrics, setMetrics] = useState<TransferMetrics>({ bytesPerSecond: null, etaSeconds: null });
  const [history, setHistory] = useState<(number | null)[]>([]);

  useEffect(() => {
    latest.current = { bytes, total, status };
    meter.current.record(bytes, status, performance.now());
    setMetrics(meter.current.read(total));
  }, [bytes, total, status]);

  useEffect(() => {
    if (status !== "downloading") return;
    setHistory([]);
    const timer = window.setInterval(() => {
      const current = latest.current;
      meter.current.record(current.bytes, current.status, performance.now());
      const next = meter.current.read(current.total);
      setMetrics(next);
      setHistory((previous) => [...previous.slice(-59), next.bytesPerSecond]);
    }, 1000);
    return () => window.clearInterval(timer);
  }, [status]);

  return { ...metrics, history };
}
