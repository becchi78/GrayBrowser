export function formatDuration(seconds: number | null): string {
  if (seconds === null) return "不明";
  const total = Math.round(seconds);
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  const mm = String(m).padStart(2, "0");
  const ss = String(s).padStart(2, "0");
  return h > 0 ? `${h}:${mm}:${ss}` : `${m}:${ss}`;
}

export function formatFileSize(bytes: number): string {
  const mb = bytes / (1024 * 1024);
  return mb >= 1024 ? `${(mb / 1024).toFixed(2)} GB` : `${mb.toFixed(1)} MB`;
}

export function formatResolution(width: number | null, height: number | null): string {
  return width !== null && height !== null ? `${width} × ${height}` : "不明";
}

export function formatBitrate(bps: number | null): string {
  if (bps === null) return "不明";
  return `${(bps / 1_000_000).toFixed(1)} Mbps`;
}

export function formatFps(fps: number | null): string {
  return fps === null ? "不明" : `${fps.toFixed(2)} fps`;
}
