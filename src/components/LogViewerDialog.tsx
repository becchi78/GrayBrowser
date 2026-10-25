import { useEffect, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { useVirtualizer } from "@tanstack/react-virtual";
import { getLogDir, readLog } from "../api";
import {
  LOG_LEVELS,
  entryMatches,
  mergeEntries,
  shouldStopFollowing,
  singleLine,
} from "../lib/logFilter";
import type { LogEntryDto } from "../types";

interface Props {
  open: boolean;
  onClose: () => void;
}

const POLL_INTERVAL_MS = 1000;
// 画面に持つ件数の上限。古い分は捨てて、長時間開きっぱなしでも重くならないようにする。
const MAX_ENTRIES = 5000;
const ROW_HEIGHT = 22;
// これより下端から離れたらユーザーが上へスクロールしたとみなす。
const FOLLOW_THRESHOLD_PX = 24;

interface Row extends LogEntryDto {
  id: number;
}

// 開くたびに内側を作り直し、ポーリングと保持データを閉じた時点で必ず破棄する。
export function LogViewerDialog({ open, onClose }: Props) {
  return open ? <LogViewerContent onClose={onClose} /> : null;
}

function LogViewerContent({ onClose }: { onClose: () => void }) {
  const [rows, setRows] = useState<Row[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [logDir, setLogDir] = useState<string | null>(null);
  const [levels, setLevels] = useState<ReadonlySet<string>>(() => new Set(LOG_LEVELS));
  const [query, setQuery] = useState("");
  const [follow, setFollow] = useState(true);
  const closeRef = useRef<HTMLButtonElement>(null);
  const scrollRef = useRef<HTMLDivElement>(null);
  const lastScrollTopRef = useRef(0);

  // 1 秒ごとの増分読み取り。前の読み取りが終わってから次を予約するので、
  // 遅い読み取りが重なることはない。閉じたら(アンマウント)必ず止める。
  useEffect(() => {
    let cancelled = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    let offset: number | null = null;
    let nextId = 0;

    async function tick() {
      try {
        const chunk = await readLog(offset);
        if (cancelled) return;
        offset = chunk.next_offset;
        setError(null);
        if (chunk.reset || chunk.entries.length > 0) {
          const incoming = chunk.entries.map((e) => ({ ...e, id: nextId++ }));
          setRows((prev) => mergeEntries(prev, incoming, chunk.reset, MAX_ENTRIES));
        }
      } catch (e) {
        if (cancelled) return;
        console.error(e);
        setError(String(e));
      }
      if (!cancelled) {
        timer = setTimeout(tick, POLL_INTERVAL_MS);
      }
    }
    void tick();

    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  }, []);

  useEffect(() => {
    let cancelled = false;
    getLogDir()
      .then((dir) => {
        if (!cancelled) setLogDir(dir);
      })
      .catch((e) => console.error(e));
    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => {
    closeRef.current?.focus();
  }, []);

  useEffect(() => {
    function onKeyDown(e: KeyboardEvent) {
      if (e.key === "Escape") {
        e.preventDefault();
        onClose();
      }
    }
    document.addEventListener("keydown", onKeyDown);
    return () => document.removeEventListener("keydown", onKeyDown);
  }, [onClose]);

  const visible = useMemo(
    () => rows.filter((r) => entryMatches(r, levels, query)),
    [rows, levels, query],
  );

  const virtualizer = useVirtualizer({
    count: visible.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => ROW_HEIGHT,
    overscan: 20,
  });

  // 追従中は、行が増えた(または追従をオンにした)ときに末尾へ寄せる。
  // 描画後の scrollHeight は増えた分を含むので、直接末尾へ送れる。
  useEffect(() => {
    const el = scrollRef.current;
    if (follow && el) {
      el.scrollTop = el.scrollHeight;
    }
  }, [follow, visible.length]);

  function handleScroll() {
    const el = scrollRef.current;
    if (!el) return;
    // 前回値は追従のオン/オフにかかわらず毎回更新する。
    const prevTop = lastScrollTopRef.current;
    lastScrollTopRef.current = el.scrollTop;
    if (!follow) return;
    // 追従による末尾への移動は下端との距離が 0 になる。縦位置が動いていない
    // (横スクロールだけの)イベントは判定から除く。
    const distance = el.scrollHeight - el.scrollTop - el.clientHeight;
    if (shouldStopFollowing(prevTop, el.scrollTop, distance, FOLLOW_THRESHOLD_PX)) {
      setFollow(false);
    }
  }

  function toggleLevel(level: string) {
    setLevels((prev) => {
      const next = new Set(prev);
      if (next.has(level)) {
        next.delete(level);
      } else {
        next.add(level);
      }
      return next;
    });
  }

  return createPortal(
    <div className="dialog-overlay">
      <div
        className="dialog log-viewer-dialog"
        role="dialog"
        aria-modal="true"
        aria-label="動作ログ"
        data-testid="log-viewer-dialog"
      >
        <h2>動作ログ</h2>
        <div className="log-viewer-controls">
          {LOG_LEVELS.map((level) => (
            <label key={level} className={`log-viewer-level log-level-${level.toLowerCase()}`}>
              <input
                type="checkbox"
                checked={levels.has(level)}
                onChange={() => toggleLevel(level)}
                data-testid={`log-viewer-level-${level}`}
              />
              {level}
            </label>
          ))}
          <input
            type="search"
            className="log-viewer-search"
            placeholder="メッセージ・モジュール名を検索"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            data-testid="log-viewer-search"
          />
          <label className="log-viewer-follow">
            <input
              type="checkbox"
              checked={follow}
              onChange={(e) => setFollow(e.target.checked)}
              data-testid="log-viewer-follow"
            />
            最新に追従
          </label>
          <span className="log-viewer-count" data-testid="log-viewer-count">
            {visible.length} / {rows.length} 件
          </span>
        </div>
        {error !== null && (
          <p className="log-viewer-error" role="alert" data-testid="log-viewer-error">
            {error}（再試行しています）
          </p>
        )}
        <div
          className="log-viewer-list"
          ref={scrollRef}
          onScroll={handleScroll}
          data-testid="log-viewer-list"
        >
          <div style={{ height: virtualizer.getTotalSize(), position: "relative" }}>
            {virtualizer.getVirtualItems().map((item) => {
              const row = visible[item.index];
              return (
                <div
                  key={row.id}
                  className="log-viewer-row"
                  title={row.message}
                  style={{
                    position: "absolute",
                    top: 0,
                    left: 0,
                    height: ROW_HEIGHT,
                    transform: `translateY(${item.start}px)`,
                  }}
                >
                  <span className={`log-viewer-row-level log-level-${row.level.toLowerCase()}`}>
                    {row.level}
                  </span>
                  {row.target !== "" && (
                    <span className="log-viewer-row-target">[{row.target}]</span>
                  )}
                  <span className="log-viewer-row-message">{singleLine(row.message)}</span>
                </div>
              );
            })}
          </div>
        </div>
        <div className="dialog-footer log-viewer-footer">
          <span className="log-viewer-dir" data-testid="log-viewer-dir">
            ログフォルダ: {logDir ?? "..."}
          </span>
          <button type="button" ref={closeRef} data-testid="log-viewer-close-btn" onClick={onClose}>
            閉じる
          </button>
        </div>
      </div>
    </div>,
    document.body,
  );
}
