import { useEffect, useState } from "react";
import { getFfmpegStatus } from "../api";

export function FfmpegNotice() {
  const [available, setAvailable] = useState<boolean | null>(null);

  useEffect(() => {
    getFfmpegStatus()
      .then((status) => setAvailable(status.available))
      .catch(() => setAvailable(false));
  }, []);

  if (available !== false) {
    return null;
  }

  return (
    <div className="ffmpeg-notice" role="alert">
      FFmpeg / FFprobe が見つかりません。PATH に追加すると、サムネイルが自動的に生成されるようになります。
    </div>
  );
}
