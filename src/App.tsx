import { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import Settings from "./pages/Settings";
import History from "./pages/History";
import Feedback from "./pages/Feedback";
import Onboarding from "./pages/Onboarding";
import RecordingOverlay from "./components/RecordingOverlay";
import "./App.css";

type AppView = "main" | "settings" | "history" | "feedback";

function App() {
  const [view, setView] = useState<AppView>("main");
  const [status, setStatus] = useState("idle");
  const [lastText, _setLastText] = useState("");
  // null = not yet checked; true/false = result from backend
  const [onboardingDone, setOnboardingDone] = useState<boolean | null>(null);

  // Crash report dialog state
  const [crashLog, setCrashLog] = useState<string | null>(null);
  const [showCrashDialog, setShowCrashDialog] = useState(false);
  const [showCrashDetails, setShowCrashDetails] = useState(false);
  const [crashSubmitting, setCrashSubmitting] = useState(false);
  const [crashSubmitResult, setCrashSubmitResult] = useState<string | null>(null);

  // Check for crash log once on mount (after onboarding check)
  useEffect(() => {
    const checkCrash = async () => {
      try {
        const log = await invoke<string | null>("check_crash_log");
        if (log) {
          setCrashLog(log);
          setShowCrashDialog(true);
        }
      } catch (e) {
        console.error("Failed to check crash log:", e);
      }
    };
    checkCrash();
  }, []);

  const handleCrashSubmit = async () => {
    if (!crashLog) return;
    setCrashSubmitting(true);
    setCrashSubmitResult(null);
    try {
      const url = await invoke<string>("submit_crash_report", { crashLog });
      setCrashSubmitResult(`Submitted: ${url}`);
      await invoke("clear_crash_log");
      setTimeout(() => {
        setShowCrashDialog(false);
        setCrashLog(null);
        setCrashSubmitResult(null);
      }, 2500);
    } catch (e) {
      setCrashSubmitResult(`Failed to submit: ${String(e)}`);
    } finally {
      setCrashSubmitting(false);
    }
  };

  const handleCrashDismiss = async () => {
    try {
      await invoke("clear_crash_log");
    } catch (e) {
      console.error("Failed to clear crash log:", e);
    }
    setShowCrashDialog(false);
    setCrashLog(null);
  };

  // Check onboarding status once on mount
  useEffect(() => {
    invoke<boolean>("is_onboarding_completed")
      .then((done) => setOnboardingDone(done))
      .catch((e) => {
        console.error("Failed to check onboarding status:", e);
        // Treat as not completed so we show onboarding
        setOnboardingDone(false);
      });
  }, []);

  useEffect(() => {
    // Poll status every 500ms
    const interval = setInterval(async () => {
      try {
        const s = await invoke<string>("get_status");
        setStatus(s);
      } catch (e) {
        console.error("Failed to get status:", e);
      }
    }, 500);
    return () => clearInterval(interval);
  }, []);

  // Still loading onboarding status — render nothing to avoid flash
  if (onboardingDone === null) {
    return null;
  }

  // Show the onboarding wizard for first-time users
  if (!onboardingDone) {
    return <Onboarding onComplete={() => setOnboardingDone(true)} />;
  }

  if (view === "settings") {
    return <Settings onBack={() => setView("main")} />;
  }

  if (view === "history") {
    return <History onBack={() => setView("main")} />;
  }

  if (view === "feedback") {
    return <Feedback onBack={() => setView("main")} />;
  }

  return (
    <main className="container">
      {/* Crash report modal — shown when a previous crash log is detected */}
      {showCrashDialog && crashLog && (
        <div className="crash-dialog-overlay">
          <div className="crash-dialog">
            <p className="crash-dialog-title">The app crashed last time</p>
            <p className="crash-dialog-desc">
              Would you like to send a crash report to help fix the issue?
            </p>

            <button
              className="crash-details-toggle"
              onClick={() => setShowCrashDetails((v) => !v)}
            >
              {showCrashDetails ? "Hide details" : "Show details"}
            </button>

            {showCrashDetails && (
              <pre className="crash-log-preview">{crashLog}</pre>
            )}

            {crashSubmitResult && (
              <p
                className={
                  crashSubmitResult.startsWith("Failed")
                    ? "crash-submit-error"
                    : "crash-submit-success"
                }
              >
                {crashSubmitResult}
              </p>
            )}

            <div className="crash-dialog-actions">
              <button onClick={handleCrashDismiss} disabled={crashSubmitting}>
                Dismiss
              </button>
              <button
                className="primary"
                onClick={handleCrashSubmit}
                disabled={crashSubmitting}
              >
                {crashSubmitting ? "Submitting…" : "Send Report"}
              </button>
            </div>
          </div>
        </div>
      )}

      <RecordingOverlay />
      <h1>Voice Typing</h1>

      <div className="status-indicator">
        <div className={`status-dot ${status}`} />
        <span className="status-text">
          {status === "idle" && "Ready - Press hotkey to speak"}
          {status === "recording" && "Recording..."}
          {status === "processing" && "Processing..."}
          {status.startsWith("error") && `Error: ${status.slice(6)}`}
        </span>
      </div>

      {lastText ? (
        <div className="last-text">
          <p className="label">Last transcription</p>
          <p className="text">{lastText}</p>
        </div>
      ) : (
        <div className="last-text" style={{ textAlign: "center", opacity: 0.6 }}>
          <p className="text" style={{ color: "var(--text-muted)", fontSize: "0.85em" }}>
            Press the hotkey to start your first voice input
          </p>
        </div>
      )}

      <div className="actions">
        <button onClick={() => setView("history")}>History</button>
        <button onClick={() => setView("settings")}>Settings</button>
        <button onClick={() => setView("feedback")}>Feedback</button>
      </div>

      <p className="hint">
        Press <kbd>Cmd</kbd>+<kbd>Shift</kbd>+<kbd>Space</kbd> to start voice input
      </p>
    </main>
  );
}

export default App;
