import { useEffect, useState } from "react";
import { checkForUpdate, getUpdateCheckOnStartup } from "../api";

// 起動直後はスキャンやサムネイル生成が動き出す時間帯なので、確認は少し待ってから行う。
const STARTUP_CHECK_DELAY_MS = 5000;

interface Props {
  onOpenDetails: () => void;
}

// 起動時の静かな更新確認。失敗(オフライン等)は何も出さず、次回起動で再び試す。
// 通知は起動中に一度だけ。「あとで」を押したら、その起動中は再び出さない。
export function UpdateNotice({ onOpenDetails }: Props) {
  const [version, setVersion] = useState<string | null>(null);
  const [dismissed, setDismissed] = useState(false);

  useEffect(() => {
    let cancelled = false;
    const timer = window.setTimeout(() => {
      void (async () => {
        try {
          if (!(await getUpdateCheckOnStartup())) {
            return;
          }
          const update = await checkForUpdate();
          if (update === null) {
            return;
          }
          const found = update.version;
          // 適用はダイアログ側で確認し直すため、ここでは Rust 側のリソースを手放す。
          await update.close().catch(() => {});
          if (!cancelled) {
            setVersion(found);
          }
        } catch (e) {
          console.warn("startup update check failed", e);
        }
      })();
    }, STARTUP_CHECK_DELAY_MS);
    return () => {
      cancelled = true;
      window.clearTimeout(timer);
    };
  }, []);

  if (version === null || dismissed) {
    return null;
  }

  return (
    <div className="update-notice" role="status" data-testid="update-notice">
      <span>新しいバージョン {version} があります</span>
      <button
        type="button"
        data-testid="update-notice-details-btn"
        onClick={() => {
          setDismissed(true);
          onOpenDetails();
        }}
      >
        詳細・更新
      </button>
      <button type="button" data-testid="update-notice-later-btn" onClick={() => setDismissed(true)}>
        あとで
      </button>
    </div>
  );
}
