import assert from "node:assert/strict";
import { test } from "node:test";
import { TransferMeter, formatRate, formatEta } from "../apps/desktop/src/transferMetrics.ts";

test("restored bytes are a baseline and ETA uses only newly transferred bytes", () => {
  const meter = new TransferMeter();
  meter.record(100_000n, "downloading", 0);
  assert.deepEqual(meter.read(120_000n), { bytesPerSecond: null, etaSeconds: null });
  meter.record(102_000n, "downloading", 1000);
  assert.deepEqual(meter.read(120_000n), { bytesPerSecond: 2000, etaSeconds: 9 });
});

test("unknown length has a speed but no ETA; completion never has negative remaining time", () => {
  const meter = new TransferMeter();
  meter.record(0n, "downloading", 0);
  meter.record(1024n, "downloading", 1000);
  assert.deepEqual(meter.read(null), { bytesPerSecond: 1024, etaSeconds: null });
  assert.equal(meter.read(1n).etaSeconds, 0);
});

test("pause, resume, retry, verification and counter resets discard stale rates", () => {
  for (const status of ["paused", "retrying", "verifying", "completed", "failed", "queued", "cancelled", "extracting"]) {
    const meter = new TransferMeter();
    meter.record(0n, "downloading", 0);
    meter.record(1000n, "downloading", 1000);
    meter.record(1000n, status, 2000);
    assert.equal(meter.read(5000n).bytesPerSecond, null);
    meter.record(1000n, "downloading", 60_000);
    assert.equal(meter.read(5000n).bytesPerSecond, null);
    meter.record(2000n, "downloading", 61_000);
    assert.equal(meter.read(5000n).bytesPerSecond, 1000);
    meter.record(0n, "downloading", 62_000);
    assert.equal(meter.read(5000n).bytesPerSecond, null);
  }
});

test("a stalled transfer reaches zero speed and an unknown ETA", () => {
  const meter = new TransferMeter();
  meter.record(0n, "downloading", 0);
  meter.record(1000n, "downloading", 1000);
  for (let now = 2000; now <= 7000; now += 1000) meter.record(1000n, "downloading", now);
  assert.deepEqual(meter.read(5000n), { bytesPerSecond: 0, etaSeconds: null });
});

test("large byte counters retain precise deltas and subsecond samples warm up", () => {
  const meter = new TransferMeter();
  const baseline = 2n ** 63n;
  meter.record(baseline, "downloading", 0);
  meter.record(baseline + 100n, "downloading", 100);
  assert.equal(meter.read(null).bytesPerSecond, null);
  meter.record(baseline + 1000n, "downloading", 1000);
  assert.equal(meter.read(baseline + 2000n).etaSeconds, 1);
});

test("human readable speed and ETA handle unavailable values", () => {
  assert.equal(formatRate(1048576), "1.0 MiB/s");
  assert.equal(formatRate(0), "0 B/s");
  assert.equal(formatRate(null), "—");
  assert.equal(formatRate(Infinity), "—");
  assert.equal(formatEta(null), "—");
  assert.equal(formatEta(0.1), "1s");
  assert.equal(formatEta(65), "1m 5s");
  assert.equal(formatEta(3665), "1h 1m");
  assert.equal(formatEta(90000), "1d 1h");
});
