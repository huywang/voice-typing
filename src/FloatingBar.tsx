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

/**
 * FloatingBar is rendered in the dedicated "floating-bar" Tauri window.
 * It polls the backend floating state every 200ms and switches between three
 * visible states:
 *   - recording  – red pulsing dot
 *   - processing – spinning orange ring
 *   - result     – injected text preview with Copy and Dismiss buttons
 *
 * The window is shown/hidden by lib.rs (recording start / error paths).
 * On successful injection the backend sets status="result"; the frontend is
 * then responsible for hiding the window after 3 seconds (or on Dismiss).
 */
function FloatingBar() {
  const [floatingState, setFloatingState] = useState<FloatingStateData>({
    status: "idle",
    text: "",
  });

  // Timer ref so we can cancel and restart the auto-dismiss on hover.
  const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  // Whether the mouse is currently hovering over the result card.
  const hoveringRef = useRef(false);

  /** Dismiss the result card: reset backend state and hide the window. */
  const dismiss = async () => {
    if (timerRef.current) {
      clearTimeout(timerRef.current);
      timerRef.current = null;
    }
    try {
      await invoke("set_floating_state", { status: "idle", text: "" });
      // Tauri hides the window from Rust via hide_floating_bar, but since we
      // transitioned via "result" (window stays open), we must hide it here.
      const { getCurrentWindow } = await import("@tauri-apps/api/window");
      await getCurrentWindow().hide();
    } catch {
      // ignore
    }
    setFloatingState({ status: "idle", text: "" });
  };

  /** Start the 3-second auto-dismiss timer (idempotent – clears any existing). */
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
          // When transitioning into "result", start the auto-dismiss timer.
          if (s.status === "result" && prev.status !== "result") {
            startTimer();
          }
          return s;
        });
      } catch {
        // ignore – backend may not be ready yet
      }
    }, 200);
    return () => clearInterval(interval);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Clean up timer on unmount.
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
    // Render an invisible placeholder so the window layout stays stable.
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
        // Fallback for environments where clipboard API is restricted.
      }
      await dismiss();
    };

    const handleMouseEnter = () => {
      hoveringRef.current = true;
      // Pause the timer while hovering.
      if (timerRef.current) {
        clearTimeout(timerRef.current);
        timerRef.current = null;
      }
    };

    const handleMouseLeave = () => {
      hoveringRef.current = false;
      // Restart the timer after the mouse leaves.
      startTimer();
    };

    return (
      <div
        className="floating-bar floating-bar--result"
        onMouseEnter={handleMouseEnter}
        onMouseLeave={handleMouseLeave}
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

  return (
    <div className={`floating-bar floating-bar--${status}`}>
      {isRecording ? (
        <div className="floating-waveform">
          <span className="floating-waveform-bar" />
          <span className="floating-waveform-bar" />
          <span className="floating-waveform-bar" />
          <span className="floating-waveform-bar" />
          <span className="floating-waveform-bar" />
        </div>
      ) : (
        <span className="floating-spinner" />
      )}
      <span className="floating-label">
        {isRecording ? "Listening..." : "Thinking..."}
      </span>
    </div>
  );
}

export default FloatingBar;
