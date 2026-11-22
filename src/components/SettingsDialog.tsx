import { useEffect, useRef, useState } from "react";
import type { FormEvent } from "react";
import {
  getDeveloperMode,
  getNasPollIntervalMinutes,
  getRememberWindowSizeEnabled,
  getRestoreViewStateEnabled,
  getUpdateCheckOnStartup,
  setDeveloperMode,
  setNasPollIntervalMinutes,
  setRememberWindowSizeEnabled,
  setRestoreViewStateEnabled,
  setUpdateCheckOnStartup,
} from "../api";
import { errorText } from "../lib/errorText";
import { DEFAULT_MINUTES, MAX_MINUTES, MIN_MINUTES, parseIntervalMinutes } from "../lib/nasInterval";
import { ModalShell } from "./ModalShell";

interface Props {
  open: boolean;
  onClose: () => void;
}

export function SettingsDialog({ open, onClose }: Props) {
  // 開くたびに内側を作り直し、前回の入力やエラーを state に残さない。
  return open ? <SettingsDialogContent onClose={onClose} /> : null;
}

function SettingsDialogContent({ onClose }: { onClose: () => void }) {
  const inputRef = useRef<HTMLInputElement>(null);
  const [text, setText] = useState("");
  const [updateCheck, setUpdateCheck] = useState(true);
  const [restoreView, setRestoreView] = useState(true);
  const [rememberWindowSize, setRememberWindowSize] = useState(true);
  const [developerMode, setDeveloperModeChecked] = useState(false);
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
    Promise.all([
      getNasPollIntervalMinutes(),
      getUpdateCheckOnStartup(),
      getRestoreViewStateEnabled(),
      getRememberWindowSizeEnabled(),
      getDeveloperMode(),
    ])
      .then(([minutes, updateCheckOnStartup, restoreViewState, rememberSize, developer]) => {
        if (!cancelled) {
          setText(String(minutes));
          setUpdateCheck(updateCheckOnStartup);
          setRestoreView(restoreViewState);
          setRememberWindowSize(rememberSize);
          setDeveloperModeChecked(developer);
          setLoading(false);
        }
      })
      .catch((e) => {
        console.error(e);
        if (!cancelled) {
          setError(`現在の設定を取得できませんでした: ${errorText(e)}`);
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
      await setRestoreViewStateEnabled(restoreView);
      await setRememberWindowSizeEnabled(rememberWindowSize);
      await setDeveloperMode(developerMode);
      if (mountedRef.current) {
        onClose();
      }
    } catch (err) {
      console.error(err);
      if (mountedRef.current) {
        setError(errorText(err));
        setSaving(false);
      }
    }
  }

  return (
    <ModalShell
      label="設定"
      className="settings-dialog"
      testId="settings-dialog"
      onDismiss={onClose}
      // 保存中は閉じない(結果が見えないまま閉じて、二重実行や中途半端な状態を作らない)。
      dismissible={!saving}
      onSubmit={handleSubmit}
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
        <p className="dialog-hint">
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
        <p className="dialog-hint">
          オフにしても、「ヘルプ &gt; 更新の確認...」からいつでも確認できます。
        </p>
      </div>
      <div className="settings-dialog-field">
        <label className="settings-dialog-check">
          <input
            type="checkbox"
            checked={restoreView}
            disabled={busy}
            data-testid="settings-restore-view-input"
            onChange={(e) => setRestoreView(e.target.checked)}
          />
          前回終了時の検索・フォルダなどの状態を、起動時に復元する
        </label>
        <p className="dialog-hint">
          検索語、フォルダ、並び順、タグ、レーティングの絞り込みを復元します。オフにすると、保存済みの状態も消去されます。
        </p>
      </div>
      <div className="settings-dialog-field">
        <label className="settings-dialog-check">
          <input
            type="checkbox"
            checked={rememberWindowSize}
            disabled={busy}
            data-testid="settings-remember-window-size-input"
            onChange={(e) => setRememberWindowSize(e.target.checked)}
          />
          前回のウィンドウの大きさを記憶する
        </label>
        <p className="dialog-hint">
          オフにすると、起動のたびに、既定の大きさで開きます。
        </p>
      </div>
      <div className="settings-dialog-field">
        <label className="settings-dialog-check">
          <input
            type="checkbox"
            checked={developerMode}
            disabled={busy}
            data-testid="settings-developer-mode-input"
            onChange={(e) => setDeveloperModeChecked(e.target.checked)}
          />
          開発者モード
        </label>
        <p className="dialog-hint">
          オンにすると、更新の確認で、安定版に加えて、公開前の先行版(動作確認用)も対象になります。先行版は不安定な場合があります。データベースの構造が変わる更新のときは、更新後の最初の起動で自動的にバックアップされます。
        </p>
      </div>
      {error !== null && (
        <p className="dialog-error dialog-error--compact" role="alert" data-testid="settings-dialog-error">
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
            setRestoreView(true);
            setRememberWindowSize(true);
            setDeveloperModeChecked(false);
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
    </ModalShell>
  );
}
