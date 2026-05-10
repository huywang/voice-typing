import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

function RecordingOverlay() {
  const [status, setStatus] = useState("idle");
  const [dots, setDots] = useState("");

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

  // Animate dots
  useEffect(() => {
    if (status !== "recording" && status !== "processing") return;
    const interval = setInterval(() => {
      setDots((d) => (d.length >= 3 ? "" : d + "."));
    }, 400);
    return () => clearInterval(interval);
  }, [status]);

  if (status !== "recording" && status !== "processing") {
    return null;
  }

  return (
    <div className="recording-overlay">
      <div className={`overlay-dot ${status}`} />
      <span className="overlay-text">
        {status === "recording" ? `Recording${dots}` : `Processing${dots}`}
      </span>
    </div>
  );
}

export default RecordingOverlay;
