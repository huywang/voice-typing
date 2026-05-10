import { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";

interface SettingsProps {
  onBack: () => void;
  onSave: () => void;
}

interface AudioDeviceInfo {
  name: string;
  is_default: boolean;
}

function Settings({ onBack, onSave }: SettingsProps) {
  const [apiKey, setApiKey] = useState("");
  const [llmEnabled, setLlmEnabled] = useState(true);
  const [autostartEnabled, setAutostartEnabled] = useState(false);
  const [soundEnabled, setSoundEnabled] = useState(true);
  const [dockVisible, setDockVisible] = useState(true);
  const [isMacos, setIsMacos] = useState(false);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState("");

  // Microphone device selection state
  const [audioDevices, setAudioDevices] = useState<AudioDeviceInfo[]>([]);
  const [selectedDevice, setSelectedDevice] = useState<string | null>(null);
  const [devicesLoading, setDevicesLoading] = useState(true);

  // Personal dictionary state
  const [dictionary, setDictionary] = useState<string[]>([]);
  const [newTerm, setNewTerm] = useState("");

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

    // Load available audio input devices.
    invoke<AudioDeviceInfo[]>("list_audio_devices")
      .then((devices) => setAudioDevices(devices))
      .catch((e) => console.error("Failed to list audio devices:", e))
      .finally(() => setDevicesLoading(false));

    // Load currently selected device (null means system default).
    invoke<string | null>("get_audio_device")
      .then((device) => setSelectedDevice(device ?? null))
      .catch(() => {
        // Non-fatal: keep null (system default).
      });

    // Detect macOS via the backend and load dock visibility setting.
    invoke<boolean>("is_macos")
      .then((macos) => {
        setIsMacos(macos);
        if (macos) {
          invoke<boolean>("get_dock_visible")
            .then((visible) => setDockVisible(visible))
            .catch(() => {
              // Non-fatal: keep the default value of true.
            });
        }
      })
      .catch(() => {
        // Non-fatal: dock toggle simply won't appear.
      });

    invoke<string[]>("get_dictionary")
      .then((terms) => setDictionary(terms))
      .catch((e) => console.error("Failed to get dictionary:", e));
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
      await invoke("set_audio_device", { deviceName: selectedDevice });
      if (isMacos) {
        await invoke("set_dock_visible", { visible: dockVisible });
      }
      onSave();
    } catch (e) {
      setError(`Failed to save: ${e}`);
    } finally {
      setSaving(false);
    }
  };

  // Add a new term to the dictionary.
  const handleAddTerm = async () => {
    const term = newTerm.trim();
    if (!term || dictionary.includes(term)) {
      setNewTerm("");
      return;
    }
    const updated = [...dictionary, term];
    setDictionary(updated);
    setNewTerm("");
    try {
      await invoke("set_dictionary", { terms: updated });
    } catch (e) {
      console.error("Failed to save dictionary:", e);
    }
  };

  // Remove a term from the dictionary.
  const handleRemoveTerm = async (term: string) => {
    const updated = dictionary.filter((t) => t !== term);
    setDictionary(updated);
    try {
      await invoke("set_dictionary", { terms: updated });
    } catch (e) {
      console.error("Failed to save dictionary:", e);
    }
  };

  // Allow pressing Enter to add a term.
  const handleTermKeyDown = (e: React.KeyboardEvent<HTMLInputElement>) => {
    if (e.key === "Enter") {
      e.preventDefault();
      handleAddTerm();
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
        <label htmlFor="microphone">Microphone</label>
        <select
          id="microphone"
          value={selectedDevice ?? ""}
          onChange={(e) =>
            setSelectedDevice(e.target.value === "" ? null : e.target.value)
          }
          disabled={devicesLoading}
        >
          <option value="">Default (system default)</option>
          {audioDevices.map((device) => (
            <option key={device.name} value={device.name}>
              {device.name}
              {device.is_default ? " (default)" : ""}
            </option>
          ))}
        </select>
        <p className="form-hint">
          Select the microphone to use for voice input.
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

      {isMacos && (
        <div className="form-group">
          <label className="checkbox-label">
            <input
              type="checkbox"
              checked={dockVisible}
              onChange={(e) => setDockVisible(e.target.checked)}
            />
            Show in Dock
          </label>
          <p className="form-hint">
            Show the app icon in the macOS Dock. When hidden, the app is
            accessible only via the menu bar icon.
          </p>
        </div>
      )}

      {/* Personal dictionary section */}
      <div className="form-group">
        <label>Personal Dictionary</label>
        <p className="form-hint">
          Add brand names, technical jargon, or any terms that should be preserved verbatim during LLM polishing.
        </p>
        <div className="dictionary-input-row">
          <input
            type="text"
            value={newTerm}
            onChange={(e) => setNewTerm(e.target.value)}
            onKeyDown={handleTermKeyDown}
            placeholder="e.g. OpenAI, Tauri, gRPC"
          />
          <button type="button" onClick={handleAddTerm} className="primary">
            Add
          </button>
        </div>
        {dictionary.length > 0 && (
          <ul className="dictionary-list">
            {dictionary.map((term) => (
              <li key={term} className="dictionary-item">
                <span className="dictionary-term">{term}</span>
                <button
                  type="button"
                  className="dictionary-remove"
                  onClick={() => handleRemoveTerm(term)}
                  aria-label={`Remove ${term}`}
                >
                  ×
                </button>
              </li>
            ))}
          </ul>
        )}
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
