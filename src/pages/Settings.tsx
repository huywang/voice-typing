import { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";

interface SettingsProps {
  onBack: () => void;
  onSave: () => void;
}

function Settings({ onBack, onSave }: SettingsProps) {
  const [apiKey, setApiKey] = useState("");
  const [llmEnabled, setLlmEnabled] = useState(true);
  const [autostartEnabled, setAutostartEnabled] = useState(false);
  const [soundEnabled, setSoundEnabled] = useState(true);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState("");

  // Load persisted toggles when the settings page mounts.
  useEffect(() => {
    invoke<boolean>("get_autostart_enabled")
      .then((enabled) => setAutostartEnabled(enabled))
      .catch((e) => console.error("Failed to get autostart state:", e));

    invoke<boolean>("get_sound_enabled")
      .then((enabled) => setSoundEnabled(enabled))
      .catch(() => {
        // Non-fatal: keep the default value of true.
      });
  }, []);

  const handleSave = async () => {
    if (!apiKey.trim()) {
      setError("Please enter your DashScope API key");
      return;
    }

    setSaving(true);
    setError("");

    try {
      await invoke("set_api_config", {
        apiKey: apiKey.trim(),
        llmEnabled,
      });
      await invoke("set_autostart_enabled", { enabled: autostartEnabled });
      await invoke("set_sound_enabled", { enabled: soundEnabled });
      onSave();
    } catch (e) {
      setError(`Failed to save: ${e}`);
    } finally {
      setSaving(false);
    }
  };

  return (
    <main className="container settings">
      <h1>Settings</h1>

      <div className="form-group">
        <label htmlFor="api-key">DashScope API Key</label>
        <input
          id="api-key"
          type="password"
          value={apiKey}
          onChange={(e) => setApiKey(e.target.value)}
          placeholder="sk-..."
        />
        <p className="form-hint">
          Get your API key from{" "}
          <a href="https://dashscope.console.aliyun.com/" target="_blank">
            Alibaba Cloud DashScope
          </a>
        </p>
      </div>

      <div className="form-group">
        <label className="checkbox-label">
          <input
            type="checkbox"
            checked={llmEnabled}
            onChange={(e) => setLlmEnabled(e.target.checked)}
          />
          Enable LLM text polishing (Qwen)
        </label>
        <p className="form-hint">
          When enabled, recognized text will be polished by Qwen LLM to remove filler words and fix punctuation.
        </p>
      </div>

      <div className="form-group">
        <label className="checkbox-label">
          <input
            type="checkbox"
            checked={autostartEnabled}
            onChange={(e) => setAutostartEnabled(e.target.checked)}
          />
          Launch at startup
        </label>
        <p className="form-hint">
          Automatically start Voice Typing when you log in.
        </p>
      </div>

      <div className="form-group">
        <label className="checkbox-label">
          <input
            type="checkbox"
            checked={soundEnabled}
            onChange={(e) => setSoundEnabled(e.target.checked)}
          />
          Enable sound effects
        </label>
        <p className="form-hint">
          Play a sound when recording starts, stops, or an error occurs.
        </p>
      </div>

      {error && <p className="error">{error}</p>}

      <div className="actions">
        <button onClick={onBack} disabled={saving}>
          Back
        </button>
        <button className="primary" onClick={handleSave} disabled={saving}>
          {saving ? "Saving..." : "Save"}
        </button>
      </div>
    </main>
  );
}

export default Settings;
