export const DECIMAL_GB = 1_000_000_000;
export const DEFAULT_QUOTA_GB = 1;
export const DEFAULT_FREE_SPACE_RESERVE_BYTES = 100 * 1024 * 1024;
export const DEFAULT_CAPTURE_RESERVATION_BYTES = 10 * 1024 * 1024;

export type QuotaRecording = {
  id: string;
  createdOrder: number;
  bytes: number;
  protected: boolean;
  status: "ready" | "evicted" | "missing";
};

export function quotaBytes(gigabytes: number): number {
  if (!Number.isFinite(gigabytes) || gigabytes <= 0) {
    throw new Error("Audio quota must be a finite positive number of decimal GB.");
  }
  return Math.floor(gigabytes * DECIMAL_GB);
}

export function selectEvictions(
  recordings: QuotaRecording[],
  quota: number,
  actualBytes: number,
  reservations: number,
  nextReservation: number,
): string[] {
  if (actualBytes + reservations + nextReservation <= quota) return [];

  const candidates = recordings
    .filter((recording) => recording.status === "ready" && !recording.protected)
    .sort((a, b) => a.createdOrder - b.createdOrder || a.id.localeCompare(b.id));
  const evicted: string[] = [];
  let remaining = actualBytes + reservations + nextReservation;
  for (const recording of candidates) {
    if (remaining <= quota) break;
    remaining -= recording.bytes;
    evicted.push(recording.id);
  }
  return remaining <= quota ? evicted : [];
}

export function formatQuotaHours(gigabytes: number, bitsPerSecond: number): string {
  const hours = (gigabytes * DECIMAL_GB * 8) / bitsPerSecond / 3600;
  return `${hours.toFixed(hours >= 10 ? 0 : 1)} hours at ${Math.round(bitsPerSecond / 1000)} kb/s`;
}
