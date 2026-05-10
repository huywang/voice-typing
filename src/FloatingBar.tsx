import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import "./FloatingBar.css";

/** Shape returned by the `get_floating_state` backend command. */
interface FloatingStateData {
  status: string;
  text: string;
}

/** Auto-dismiss delay in milliseconds. */
const AUTO_DISMISS_MS = 3000;

/** Maximum characters shown in the result preview before truncation. */
const MAX_PREVIEW_CHARS = 80;

function FloatingBar() {
  const [floatingState, setFloatingState] = useState<FloatingStateData>({
    status: "idle",
    text: "",
  });

  const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const hoveringRef = useRef(false);

  const dismiss = async () => {
    if (timerRef.current) {
      clearTimeout(timerRef.current);
      timerRef.current = null;
    }
    try {
      await invoke("set_floating_state", { status: "idle", text: "" });
      const { getCurrentWindow } = await import("@tauri-apps/api/window");
      await getCurrentWindow().hide();
    } catch {
      // ignore
    }
    setFloatingState({ status: "idle", text: "" });
  };

  const startTimer = () => {
    if (timerRef.current) clearTimeout(timerRef.current);
    timerRef.current = setTimeout(() => {
      if (!hoveringRef.current) {
        dismiss();
      }
    }, AUTO_DISMISS_MS);
  };

  // Poll backend state every 200ms.
  useEffect(() => {
    const interval = setInterval(async () => {
      try {
        const s = await invoke<FloatingStateData>("get_floating_state");
        setFloatingState((prev) => {
          if (s.status === "result" && prev.status !== "result") {
            startTimer();
          }
          return s;
        });
      } catch {
        // ignore
      }
    }, 200);
    return () => clearInterval(interval);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    return () => {
      if (timerRef.current) clearTimeout(timerRef.current);
    };
  }, []);

  const { status, text } = floatingState;
  const isRecording = status === "recording";
  const isProcessing = status === "processing";
  const isResult = status === "result";

  if (!isRecording && !isProcessing && !isResult) {
    return <div className="floating-bar floating-bar--hidden" />;
  }

  if (isResult) {
    const preview =
      text.length > MAX_PREVIEW_CHARS
        ? text.slice(0, MAX_PREVIEW_CHARS) + "…"
        : text;

    const handleCopy = async () => {
      try {
        await navigator.clipboard.writeText(text);
      } catch {
        // fallback
      }
      await dismiss();
    };

    return (
      <div
        className="floating-bar floating-bar--result"
        onMouseEnter={() => {
          hoveringRef.current = true;
          if (timerRef.current) {
            clearTimeout(timerRef.current);
            timerRef.current = null;
          }
        }}
        onMouseLeave={() => {
          hoveringRef.current = false;
          startTimer();
        }}
      >
        <span className="floating-result-icon">✓</span>
        <span className="floating-result-text">{preview}</span>
        <div className="floating-result-actions">
          <button className="floating-btn floating-btn--copy" onClick={handleCopy}>
            Copy
          </button>
          <button className="floating-btn floating-btn--dismiss" onClick={dismiss}>
            ✕
          </button>
        </div>
      </div>
    );
  }

  // Recording / Processing state
  // Layout: label on left, waveform/spinner on right (matching reference design)
  return (
    <div className={`floating-bar floating-bar--${status}`}>
      <span className="floating-label">
        {isRecording ? "语音输入" : "识别中..."}
      </span>
      {isRecording ? (
        <div className="floating-waveform">
          <span className="floating-waveform-bar" />
          <span className="floating-waveform-bar" />
          <span className="floating-waveform-bar" />
          <span className="floating-waveform-bar" />
        </div>
      ) : (
        <span className="floating-spinner" />
      )}
    </div>
  );
}

export default FloatingBar;
