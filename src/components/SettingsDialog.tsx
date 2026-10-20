import { useEffect, useRef, useState } from "react";
import type { FormEvent } from "react";
import { createPortal } from "react-dom";
import {
  getNasPollIntervalMinutes,
  getUpdateCheckOnStartup,
  setNasPollIntervalMinutes,
  setUpdateCheckOnStartup,
} from "../api";
import { DEFAULT_MINUTES, MAX_MINUTES, MIN_MINUTES, parseIntervalMinutes } from "../lib/nasInterval";

interface Props {
  open: boolean;
  onClose: () => void;
}

function errorMessage(e: unknown): string {
  if (typeof e === "string") {
    return e;
  }
  return e instanceof Error ? e.message : String(e);
}

export function SettingsDialog({ open, onClose }: Props) {
  // 開くたびに内側を作り直し、前回の入力やエラーを state に残さない。
  return open ? <SettingsDialogContent onClose={onClose} /> : null;
}

function SettingsDialogContent({ onClose }: { onClose: () => void }) {
  const inputRef = useRef<HTMLInputElement>(null);
  const [text, setText] = useState("");
  const [updateCheck, setUpdateCheck] = useState(true);
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // 保存の完了後に、アンマウント済みなら state を更新しないための印。
  const mountedRef = useRef(true);
  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
    };
  }, []);

  useEffect(() => {
    // 閉じた後に取得結果が遅れて反映されないようにする。
    let cancelled = false;
    Promise.all([getNasPollIntervalMinutes(), getUpdateCheckOnStartup()])
      .then(([minutes, updateCheckOnStartup]) => {
        if (!cancelled) {
          setText(String(minutes));
          setUpdateCheck(updateCheckOnStartup);
          setLoading(false);
        }
      })
      .catch((e) => {
        console.error(e);
        if (!cancelled) {
          setError(`現在の設定を取得できませんでした: ${errorMessage(e)}`);
          setLoading(false);
        }
      });
    return () => {
      cancelled = true;
    };
  }, []);

  const busy = loading || saving;

  // disabled の要素にはフォーカスできないため、取得が終わって有効になってから当てる。
  useEffect(() => {
    if (!loading) {
      inputRef.current?.focus();
      inputRef.current?.select();
    }
  }, [loading]);

  // 保存中は閉じない(結果が見えないまま閉じて、二重実行や中途半端な状態を作らない)。
  useEffect(() => {
    function onKeyDown(e: KeyboardEvent) {
      if (e.key === "Escape" && !saving) {
        e.preventDefault();
        onClose();
      }
    }
    document.addEventListener("keydown", onKeyDown);
    return () => document.removeEventListener("keydown", onKeyDown);
  }, [onClose, saving]);

  async function handleSubmit(e: FormEvent) {
    e.preventDefault();
    if (busy) {
      return;
    }
    // 範囲の検証はバックエンドが正。ここは即時のフィードバック用。
    const parsed = parseIntervalMinutes(text);
    if (!parsed.ok) {
      setError(parsed.reason);
      return;
    }
    setError(null);
    setSaving(true);
    try {
      await setNasPollIntervalMinutes(parsed.minutes);
      await setUpdateCheckOnStartup(updateCheck);
      if (mountedRef.current) {
        onClose();
      }
    } catch (err) {
      console.error(err);
      if (mountedRef.current) {
        setError(errorMessage(err));
        setSaving(false);
      }
    }
  }

  return createPortal(
    <div className="dialog-overlay" onClick={saving ? undefined : onClose}>
      <form
        className="dialog settings-dialog"
        role="dialog"
        aria-modal="true"
        aria-label="設定"
        data-testid="settings-dialog"
        onClick={(e) => e.stopPropagation()}
        onSubmit={handleSubmit}
        noValidate
      >
        <h2>設定</h2>
        <div className="settings-dialog-field">
          <label htmlFor="settings-nas-interval">NAS・ネットワークドライブの確認間隔</label>
          <div className="settings-dialog-input-row">
            <input
              id="settings-nas-interval"
              ref={inputRef}
              className="settings-dialog-input"
              type="text"
              inputMode="numeric"
              autoComplete="off"
              value={text}
              disabled={busy}
              data-testid="settings-nas-interval-input"
              onChange={(e) => setText(e.target.value)}
            />
            <span>分</span>
          </div>
          <p className="settings-dialog-hint">
            {MIN_MINUTES}〜{MAX_MINUTES} 分。ローカルドライブは、変更を検知して即座に反映するため、この設定は関係しません。
          </p>
        </div>
        <div className="settings-dialog-field">
          <label className="settings-dialog-check">
            <input
              type="checkbox"
              checked={updateCheck}
              disabled={busy}
              data-testid="settings-update-check-input"
              onChange={(e) => setUpdateCheck(e.target.checked)}
            />
            起動時に更新を確認する
          </label>
          <p className="settings-dialog-hint">
            オフにしても、「ヘルプ &gt; 更新の確認...」からいつでも確認できます。
          </p>
        </div>
        {error !== null && (
          <p className="settings-dialog-error" role="alert" data-testid="settings-dialog-error">
            {error}
          </p>
        )}
        <div className="dialog-footer">
          <button
            type="button"
            disabled={busy}
            data-testid="settings-dialog-reset-btn"
            onClick={() => {
              setText(String(DEFAULT_MINUTES));
              setUpdateCheck(true);
              setError(null);
            }}
          >
            既定に戻す
          </button>
          <div className="settings-dialog-actions">
            <button type="button" disabled={busy} data-testid="settings-dialog-cancel-btn" onClick={onClose}>
              キャンセル
            </button>
            <button type="submit" disabled={busy} data-testid="settings-dialog-save-btn">
              保存
            </button>
          </div>
        </div>
      </form>
    </div>,
    document.body,
  );
}
