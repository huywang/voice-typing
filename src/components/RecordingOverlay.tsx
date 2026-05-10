import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

function RecordingOverlay() {
  const [status, setStatus] = useState("idle");

  useEffect(() => {
    const interval = setInterval(async () => {
      try {
        const s = await invoke<string>("get_status");
        setStatus(s);
      } catch {
        // ignore
      }
    }, 200);
    return () => clearInterval(interval);
  }, []);

  if (status !== "recording" && status !== "processing") {
    return null;
  }

  return (
    <div className="recording-overlay">
      <span className="overlay-label">
        {status === "recording" ? "语音输入" : "识别中..."}
      </span>
      {status === "recording" ? (
        <div className="overlay-waveform">
          <span className="overlay-wave-bar" />
          <span className="overlay-wave-bar" />
          <span className="overlay-wave-bar" />
          <span className="overlay-wave-bar" />
        </div>
      ) : (
        <span className="overlay-spinner" />
      )}
    </div>
  );
}

export default RecordingOverlay;
