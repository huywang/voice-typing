import { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import Settings from "./pages/Settings";
import History from "./pages/History";
import Onboarding from "./pages/Onboarding";
import RecordingOverlay from "./components/RecordingOverlay";
import "./App.css";

type AppView = "main" | "settings" | "history";

function App() {
  const [view, setView] = useState<AppView>("main");
  const [status, setStatus] = useState("idle");
  const [lastText, _setLastText] = useState("");
  // null = not yet checked; true/false = result from backend
  const [onboardingDone, setOnboardingDone] = useState<boolean | null>(null);

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

  return (
    <main className="container">
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

      {lastText && (
        <div className="last-text">
          <p className="label">Last transcription:</p>
          <p className="text">{lastText}</p>
        </div>
      )}

      <div className="actions">
        <button onClick={() => setView("history")}>History</button>
        <button onClick={() => setView("settings")}>Settings</button>
      </div>

      <p className="hint">
        Use <kbd>Ctrl+Shift+Space</kbd> (or <kbd>Cmd+Shift+Space</kbd> on Mac) to start voice input
      </p>
    </main>
  );
}

export default App;
