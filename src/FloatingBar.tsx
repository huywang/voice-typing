import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import "./FloatingBar.css";

/**
 * FloatingBar is rendered in the dedicated "floating-bar" Tauri window.
 * It polls the backend status every 200ms and shows a pill-shaped indicator
 * while recording or processing.  The window itself is controlled (shown/hidden)
 * by lib.rs, so this component always renders something visible.
 */
function FloatingBar() {
  const [status, setStatus] = useState("idle");

  useEffect(() => {
    const interval = setInterval(async () => {
      try {
        const s = await invoke<string>("get_status");
        setStatus(s);
      } catch {
        // ignore – backend may not be ready yet
      }
    }, 200);
    return () => clearInterval(interval);
  }, []);

  const isRecording = status === "recording";
  const isProcessing = status === "processing";

  if (!isRecording && !isProcessing) {
    // Render an invisible placeholder so the window layout stays stable.
    return <div className="floating-bar floating-bar--hidden" />;
  }

  return (
    <div className={`floating-bar floating-bar--${status}`}>
      <span className={`floating-dot floating-dot--${status}`} />
      <span className="floating-label">
        {isRecording ? "Recording..." : "Processing..."}
      </span>
    </div>
  );
}

export default FloatingBar;
