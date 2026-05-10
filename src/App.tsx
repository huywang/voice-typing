import { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import Settings from "./pages/Settings";
import RecordingOverlay from "./components/RecordingOverlay";
import "./App.css";

type AppView = "main" | "settings";

function App() {
  const [view, setView] = useState<AppView>("main");
  const [status, setStatus] = useState("idle");
  const [lastText, _setLastText] = useState("");
  const [configured, setConfigured] = useState(false);

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

  // Show settings on first run if not configured
  useEffect(() => {
    if (!configured) {
      setView("settings");
    }
  }, [configured]);

  const handleConfigured = () => {
    setConfigured(true);
    setView("main");
  };

  if (view === "settings") {
    return <Settings onBack={() => setView("main")} onSave={handleConfigured} />;
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
        <button onClick={() => setView("settings")}>Settings</button>
      </div>

      <p className="hint">
        Use <kbd>Ctrl+Shift+Space</kbd> (or <kbd>Cmd+Shift+Space</kbd> on Mac) to start voice input
      </p>
    </main>
  );
}

export default App;
