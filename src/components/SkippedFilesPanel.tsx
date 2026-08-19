import { useEffect, useState } from "react";
import { listSkippedFiles } from "../api";
import type { SkippedFileDto } from "../types";

// The only reason code emitted today (crates/gb-core/src/scan_pipeline.rs),
// but kept as a lookup with a fallback so a future reason doesn't render
// blank.
const REASON_LABELS: Record<string, string> = {
  machine_dependent_char: "機種依存文字を含むファイル名",
};

interface Props {
  refreshKey: number;
  // Visibility is driven by the StatusBar badge (a single "which panel is
  // open" selection), not this component's own toggle button. The
  // component stays mounted regardless of `open` so its fetch effect below
  // keeps the badge's count fresh in the background even while the panel
  // is closed.
  open: boolean;
  // Reports the current count up to StatusBar for the badge label.
  onCountChange?: (count: number) => void;
}

export function SkippedFilesPanel({ refreshKey, open, onCountChange }: Props) {
  const [files, setFiles] = useState<SkippedFileDto[]>([]);

  useEffect(() => {
    listSkippedFiles()
      .then((f) => {
        setFiles(f);
        onCountChange?.(f.length);
      })
      .catch(console.error);
    // `onCountChange` is always a stable useState setter passed by
    // StatusBar, so including it here does not cause extra refetches.
  }, [refreshKey, onCountChange]);

  if (!open) {
    return null;
  }

  return (
    <section className="skipped-files-panel status-panel" data-testid="status-panel">
      {files.length === 0 ? (
        <p>未登録ファイルはありません。</p>
      ) : (
        <>
          <p className="skipped-files-guidance">
            ファイル名を修正すると、次回のスキャンで通常どおり登録されます。
          </p>
          <ul className="skipped-files-list">
            {files.map((f) => (
              <li key={f.id}>
                <span className="skipped-file-name">{f.file_name}</span>
                <span className="skipped-file-reason">
                  {REASON_LABELS[f.reason] ?? f.reason}
                  {f.detected_char ? `（検出文字: ${f.detected_char}）` : ""}
                </span>
              </li>
            ))}
          </ul>
        </>
      )}
    </section>
  );
}
