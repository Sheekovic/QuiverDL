// Runtime-only measurements: restored bytes are never counted as new traffic.
export type TransferMetrics = { bytesPerSecond: number | null; etaSeconds: number | null };
type Sample = { bytes: bigint; at: number };

export class TransferMeter {
  private samples: Sample[] = [];
  private status = "";

  record(bytes: bigint, status: string, now: number): void {
    const previous = this.samples[this.samples.length - 1];
    if (status !== this.status || (previous && (bytes < previous.bytes || now < previous.at))) {
      this.samples = [];
    }
    this.status = status;
    if (status !== "downloading") {
      this.samples = [];
      return;
    }
    this.samples.push({ bytes, at: now });
    // Keep one anchor before the rolling window, including during a stall.
    while (this.samples.length > 2 && this.samples[1].at <= now - 5000) this.samples.shift();
  }

  read(total: bigint | null): TransferMetrics {
    const first = this.samples[0];
    const last = this.samples[this.samples.length - 1];
    if (!first || !last || last.at - first.at < 500) {
      return { bytesPerSecond: null, etaSeconds: null };
    }
    const bytesPerSecond = Number(last.bytes - first.bytes) * 1000 / (last.at - first.at);
    const remaining = total === null ? null : total > last.bytes ? total - last.bytes : 0n;
    const estimate = remaining === null || bytesPerSecond <= 0 ? null : Number(remaining) / bytesPerSecond;
    return {
      bytesPerSecond: Number.isFinite(bytesPerSecond) ? bytesPerSecond : null,
      etaSeconds: estimate !== null && Number.isFinite(estimate) ? estimate : null,
    };
  }
}

export function formatRate(rate: number | null): string {
  if (rate === null || !Number.isFinite(rate) || rate < 0) return "—";
  const units = ["B/s", "KiB/s", "MiB/s", "GiB/s", "TiB/s"];
  let unit = 0;
  while (rate >= 1024 && unit < units.length - 1) { rate /= 1024; unit += 1; }
  return `${rate.toFixed(unit === 0 ? 0 : 1)} ${units[unit]}`;
}

export function formatEta(seconds: number | null): string {
  if (seconds === null || !Number.isFinite(seconds) || seconds < 0) return "—";
  const whole = Math.ceil(seconds);
  if (whole < 60) return `${whole}s`;
  const minutes = Math.floor(whole / 60);
  if (minutes < 60) return `${minutes}m ${whole % 60}s`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours}h ${minutes % 60}m`;
  return `${Math.floor(hours / 24)}d ${hours % 24}h`;
}
