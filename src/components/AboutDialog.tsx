import { useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { getVersion } from "@tauri-apps/api/app";

interface Props {
  open: boolean;
  onClose: () => void;
}

// window.alert は使わない(ConfirmDialog と同様、ESLint で禁止している)。
// バージョンは tauri.conf.json の値を実行時に取得し、リリースごとに
// フロントエンドのコードを書き換えなくて済むようにする。
export function AboutDialog({ open, onClose }: Props) {
  // 開くたびに内側を作り直し、前回の取得結果を state に残さない。
  return open ? <AboutDialogContent onClose={onClose} /> : null;
}

function AboutDialogContent({ onClose }: { onClose: () => void }) {
  const closeRef = useRef<HTMLButtonElement>(null);
  const [version, setVersion] = useState<string | null>(null);
  const [versionFailed, setVersionFailed] = useState(false);

  useEffect(() => {
    // 閉じた後に取得結果が遅れて反映されないようにする。
    let cancelled = false;
    getVersion()
      .then((v) => {
        if (!cancelled) {
          setVersion(v);
        }
      })
      .catch((e) => {
        console.error(e);
        if (!cancelled) {
          setVersionFailed(true);
        }
      });
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

  let versionText = "取得中...";
  if (versionFailed) {
    versionText = "取得できませんでした";
  } else if (version !== null) {
    versionText = `バージョン ${version}`;
  }

  return createPortal(
    <div className="dialog-overlay" onClick={onClose}>
      <div
        className="dialog about-dialog"
        role="dialog"
        aria-modal="true"
        aria-label="バージョン情報"
        data-testid="about-dialog"
        onClick={(e) => e.stopPropagation()}
      >
        <h2>GrayBrowser</h2>
        <p className="about-dialog-version" data-testid="about-dialog-version">
          {versionText}
        </p>
        <div className="dialog-footer about-dialog-footer">
          <button type="button" ref={closeRef} data-testid="about-dialog-close-btn" onClick={onClose}>
            閉じる
          </button>
        </div>
      </div>
    </div>,
    document.body,
  );
}
