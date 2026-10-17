import { useEffect, useState } from "react";
import { getThumbnailProgress } from "../api";
import { onThumbnailProgress } from "../events";
import { formatThumbnailProgress } from "../lib/thumbnailProgress";
import type { ThumbnailProgress } from "../types";
import { DuplicateGroupsPanel } from "./DuplicateGroupsPanel";
import { GenerationFailuresPanel } from "./GenerationFailuresPanel";
import { SkippedFilesPanel } from "./SkippedFilesPanel";

// The always-visible status bar. Its three badges are the *only* visible
// trigger for the existing SkippedFilesPanel/DuplicateGroupsPanel/
// GenerationFailuresPanel content -- each panel's own toggle button was
// removed in favor of this (see each component's own comment). Only one
// panel can be open at a time (`openPanel`): a single popover slot, not
// three independently-toggleable ones.
//
// All three panel components are mounted unconditionally here (not just
// when open) so their fetch effects / event subscriptions keep the badge
// counts below current even while their own panel is closed -- each
// component itself decides whether to render its section markup (`open`
// prop) or nothing (`null`), per each component's own comment.
type PanelKind = "unregistered" | "duplicates" | "failed";

interface Props {
  refreshKey: number;
}

export function StatusBar({ refreshKey }: Props) {
  const [openPanel, setOpenPanel] = useState<PanelKind | null>(null);
  const [unregisteredCount, setUnregisteredCount] = useState(0);
  const [duplicatesCount, setDuplicatesCount] = useState(0);
  const [failedCount, setFailedCount] = useState(0);
  const [thumbnailProgress, setThumbnailProgress] =
    useState<ThumbnailProgress | null>(null);

  useEffect(() => {
    // Generation can start before this subscription attaches (startup queue),
    // so read the current value once; events take over after that. A read
    // that resolves after an event must not overwrite the newer value.
    let received = false;
    const unsubscribe = onThumbnailProgress((p) => {
      received = true;
      setThumbnailProgress(p);
    });
    getThumbnailProgress()
      .then((p) => {
        if (!received) setThumbnailProgress(p);
      })
      .catch(() => {});
    return unsubscribe;
  }, []);

  const thumbnailProgressText = thumbnailProgress
    ? formatThumbnailProgress(thumbnailProgress)
    : null;

  function toggle(kind: PanelKind) {
    setOpenPanel((prev) => (prev === kind ? null : kind));
  }

  return (
    <div className="status-bar" data-testid="status-bar">
      <div className="status-bar-badges">
        <button
          type="button"
          className="status-badge"
          data-testid="status-badge-unregistered"
          aria-pressed={openPanel === "unregistered"}
          onClick={() => toggle("unregistered")}
        >
          未登録 {unregisteredCount}
        </button>
        <button
          type="button"
          className="status-badge"
          data-testid="status-badge-duplicates"
          aria-pressed={openPanel === "duplicates"}
          onClick={() => toggle("duplicates")}
        >
          重複 {duplicatesCount}
        </button>
        <button
          type="button"
          className="status-badge"
          data-testid="status-badge-failed"
          aria-pressed={openPanel === "failed"}
          onClick={() => toggle("failed")}
        >
          生成失敗 {failedCount}
        </button>
        {thumbnailProgressText !== null && (
          <span
            className="status-thumbnail-progress"
            data-testid="status-thumbnail-progress"
          >
            {thumbnailProgressText}
          </span>
        )}
      </div>
      <SkippedFilesPanel
        refreshKey={refreshKey}
        open={openPanel === "unregistered"}
        onCountChange={setUnregisteredCount}
      />
      <DuplicateGroupsPanel
        refreshKey={refreshKey}
        open={openPanel === "duplicates"}
        onCountChange={setDuplicatesCount}
      />
      <GenerationFailuresPanel
        refreshKey={refreshKey}
        open={openPanel === "failed"}
        onCountChange={setFailedCount}
      />
    </div>
  );
}
