import { useState, useEffect, useRef, useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";

interface SettingsProps {
  onBack: () => void;
}

interface AudioDeviceInfo {
  name: string;
  is_default: boolean;
}

// Available tab identifiers
type Tab = "general" | "ai" | "shortcuts" | "about";

// Per-field save status for inline feedback
type FieldStatus = "saved" | "error" | null;

// Fixed shortcut definitions displayed on the Shortcuts tab
const SHORTCUTS: { label: string; keys: string[] }[] = [
  { label: "Push-to-Talk", keys: ["Cmd", "Shift", "Space"] },
  { label: "Paste Last", keys: ["Cmd", "Alt", "V"] },
  { label: "AI Revert", keys: ["Cmd", "Alt", "Z"] },
  { label: "Translation", keys: ["Cmd", "Shift", "T"] },
];

// Duration (ms) before inline save status fades away
const STATUS_CLEAR_DELAY = 2000;

function Settings({ onBack }: SettingsProps) {
  const [activeTab, setActiveTab] = useState<Tab>("general");

  // General tab state
  const [apiKey, setApiKey] = useState("");
  const [autostartEnabled, setAutostartEnabled] = useState(false);
  const [soundEnabled, setSoundEnabled] = useState(true);
  const [dockVisible, setDockVisible] = useState(true);
  const [isMacos, setIsMacos] = useState(false);

  // Microphone device selection state
  const [audioDevices, setAudioDevices] = useState<AudioDeviceInfo[]>([]);
  const [selectedDevice, setSelectedDevice] = useState<string | null>(null);
  const [devicesLoading, setDevicesLoading] = useState(true);

  // Push-to-talk hotkey state
  const [hotkey, setHotkey] = useState("CmdOrCtrl+Shift+Space");
  const [hotkeySaving, setHotkeySaving] = useState(false);
  const [hotkeyError, setHotkeyError] = useState("");
  const [hotkeySaved, setHotkeySaved] = useState(false);

  // API key connection test state
  const [testing, setTesting] = useState(false);
  const [testResult, setTestResult] = useState<{ ok: boolean; message: string } | null>(null);

  // AI tab state
  const [llmEnabled, setLlmEnabled] = useState(true);

  // Translation target language state
  const [translationTarget, setTranslationTarget] = useState("English");

  // Personal dictionary state
  const [dictionary, setDictionary] = useState<string[]>([]);
  const [newTerm, setNewTerm] = useState("");

  // Per-field inline save status map
  const [fieldStatus, setFieldStatus] = useState<Record<string, FieldStatus>>({});

  // Timers for auto-clearing field status after STATUS_CLEAR_DELAY
  const statusTimers = useRef<Record<string, ReturnType<typeof setTimeout>>>({});

  // Set a field's status and schedule auto-clear
  const setStatus = useCallback((field: string, status: FieldStatus) => {
    setFieldStatus((prev) => ({ ...prev, [field]: status }));

    // Clear any existing timer for this field
    if (statusTimers.current[field]) {
      clearTimeout(statusTimers.current[field]);
    }

    if (status !== null) {
      statusTimers.current[field] = setTimeout(() => {
        setFieldStatus((prev) => ({ ...prev, [field]: null }));
      }, STATUS_CLEAR_DELAY);
    }
  }, []);

  // Cleanup all timers on unmount
  useEffect(() => {
    const timers = statusTimers.current;
    return () => {
      Object.values(timers).forEach(clearTimeout);
    };
  }, []);

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

    // Load persisted push-to-talk hotkey.
    invoke<string>("get_hotkey")
      .then((hk) => setHotkey(hk))
      .catch(() => {
        // Non-fatal: keep the default value.
      });

    // Load persisted translation target language.
    invoke<string>("get_translation_target")
      .then((lang) => setTranslationTarget(lang))
      .catch(() => {
        // Non-fatal: keep the default "English".
      });

    // API key is write-only for security; field intentionally starts empty on
    // each settings open. The user only needs to re-enter it to change it.
  }, []);

  // --- Instant-save handlers for toggle/checkbox fields ---

  const handleAutostartChange = async (checked: boolean) => {
    setAutostartEnabled(checked);
    try {
      await invoke("set_autostart_enabled", { enabled: checked });
      setStatus("autostart", "saved");
    } catch {
      setAutostartEnabled(!checked); // revert on failure
      setStatus("autostart", "error");
    }
  };

  const handleSoundChange = async (checked: boolean) => {
    setSoundEnabled(checked);
    try {
      await invoke("set_sound_enabled", { enabled: checked });
      setStatus("sound", "saved");
    } catch {
      setSoundEnabled(!checked); // revert on failure
      setStatus("sound", "error");
    }
  };

  const handleDockChange = async (checked: boolean) => {
    setDockVisible(checked);
    try {
      await invoke("set_dock_visible", { visible: checked });
      setStatus("dock", "saved");
    } catch {
      setDockVisible(!checked); // revert on failure
      setStatus("dock", "error");
    }
  };

  const handleLlmChange = async (checked: boolean) => {
    setLlmEnabled(checked);
    try {
      // set_api_config requires both apiKey and llmEnabled; pass current apiKey
      await invoke("set_api_config", { apiKey: apiKey.trim(), llmEnabled: checked });
      setStatus("llm", "saved");
    } catch {
      setLlmEnabled(!checked); // revert on failure
      setStatus("llm", "error");
    }
  };

  const handleDeviceChange = async (deviceName: string | null) => {
    setSelectedDevice(deviceName);
    try {
      await invoke("set_audio_device", { deviceName });
      setStatus("device", "saved");
    } catch {
      setStatus("device", "error");
    }
  };

  // --- Blur-save handlers for text fields ---

  const handleApiKeyBlur = async () => {
    const trimmed = apiKey.trim();
    if (!trimmed) {
      setStatus("apiKey", "error");
      return;
    }
    try {
      await invoke("set_api_config", { apiKey: trimmed, llmEnabled });
      setStatus("apiKey", "saved");
    } catch {
      setStatus("apiKey", "error");
    }
  };

  // Send a silent test WAV to verify the API key is valid.
  const handleTestApiKey = async () => {
    const trimmed = apiKey.trim();
    if (!trimmed) {
      setTestResult({ ok: false, message: "Please enter an API key first." });
      return;
    }
    setTesting(true);
    setTestResult(null);
    try {
      await invoke<string>("test_api_key", { apiKey: trimmed });
      setTestResult({ ok: true, message: "Connected!" });
    } catch (e) {
      setTestResult({ ok: false, message: String(e) });
    } finally {
      setTesting(false);
    }
  };

  const handleTranslationTargetBlur = async () => {
    const trimmed = translationTarget.trim();
    if (!trimmed) {
      setStatus("translationTarget", "error");
      return;
    }
    try {
      await invoke("set_translation_target", { language: trimmed });
      setStatus("translationTarget", "saved");
    } catch {
      setStatus("translationTarget", "error");
    }
  };

  // --- Hotkey save (requires explicit Apply button + restart) ---
  const handleSaveHotkey = async () => {
    const trimmed = hotkey.trim();
    if (!trimmed) {
      setHotkeyError("Hotkey cannot be empty.");
      return;
    }
    setHotkeySaving(true);
    setHotkeyError("");
    setHotkeySaved(false);
    try {
      await invoke("set_hotkey", { hotkey: trimmed });
      setHotkey(trimmed);
      setHotkeySaved(true);
    } catch (e) {
      setHotkeyError(`Failed to save hotkey: ${e}`);
    } finally {
      setHotkeySaving(false);
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

  // Render a small inline status indicator (checkmark or X) for a given field
  const renderFieldStatus = (field: string) => {
    const status = fieldStatus[field];
    if (!status) return null;
    return (
      <span
        className={`field-status field-status--${status}`}
        aria-live="polite"
      >
        {status === "saved" ? "✓" : "✗"}
      </span>
    );
  };

  return (
    <main className="container settings">
      <h1>Settings</h1>

      <div className="settings-tabs">
        {/* Tab bar */}
        <div className="tab-bar" role="tablist">
          {(["general", "ai", "shortcuts", "about"] as Tab[]).map((tab) => (
            <button
              key={tab}
              role="tab"
              aria-selected={activeTab === tab}
              className={`tab-btn${activeTab === tab ? " active" : ""}`}
              onClick={() => setActiveTab(tab)}
            >
              {tab.charAt(0).toUpperCase() + tab.slice(1)}
            </button>
          ))}
        </div>

        {/* General tab — API key, microphone, launch settings */}
        {activeTab === "general" && (
          <div className="tab-panel" role="tabpanel">
            <div className="form-group">
              <label htmlFor="api-key">
                DashScope API Key
                {renderFieldStatus("apiKey")}
              </label>
              <div className="dictionary-input-row">
                <input
                  id="api-key"
                  type="password"
                  value={apiKey}
                  onChange={(e) => {
                    setApiKey(e.target.value);
                    // Clear previous test result when the key changes.
                    setTestResult(null);
                  }}
                  onBlur={handleApiKeyBlur}
                  placeholder="sk-..."
                />
                <button
                  type="button"
                  onClick={handleTestApiKey}
                  disabled={testing}
                >
                  {testing ? "Testing..." : "Test"}
                </button>
              </div>
              {testResult && (
                <p
                  className="form-hint"
                  style={{ color: testResult.ok ? "var(--status-idle)" : "var(--status-error, #e55)" }}
                  aria-live="polite"
                >
                  {testResult.message}
                </p>
              )}
              <p className="form-hint">
                Get your API key from{" "}
                <a href="https://dashscope.console.aliyun.com/" target="_blank">
                  Alibaba Cloud DashScope
                </a>
                . Saved automatically when you leave this field.
              </p>
            </div>

            <div className="form-group">
              <label htmlFor="microphone">
                Microphone
                {renderFieldStatus("device")}
              </label>
              <select
                id="microphone"
                value={selectedDevice ?? ""}
                onChange={(e) =>
                  handleDeviceChange(
                    e.target.value === "" ? null : e.target.value,
                  )
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
                  checked={autostartEnabled}
                  onChange={(e) => handleAutostartChange(e.target.checked)}
                />
                Launch at startup
                {renderFieldStatus("autostart")}
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
                  onChange={(e) => handleSoundChange(e.target.checked)}
                />
                Enable sound effects
                {renderFieldStatus("sound")}
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
                    onChange={(e) => handleDockChange(e.target.checked)}
                  />
                  Show in Dock
                  {renderFieldStatus("dock")}
                </label>
                <p className="form-hint">
                  Show the app icon in the macOS Dock. When hidden, the app is
                  accessible only via the menu bar icon.
                </p>
              </div>
            )}
          </div>
        )}

        {/* AI tab — LLM polishing, translation target, personal dictionary */}
        {activeTab === "ai" && (
          <div className="tab-panel" role="tabpanel">
            <div className="form-group">
              <label className="checkbox-label">
                <input
                  type="checkbox"
                  checked={llmEnabled}
                  onChange={(e) => handleLlmChange(e.target.checked)}
                />
                Enable LLM text polishing (Qwen)
                {renderFieldStatus("llm")}
              </label>
              <p className="form-hint">
                When enabled, recognized text will be polished by Qwen LLM to
                remove filler words and fix punctuation.
              </p>
            </div>

            <div className="form-group">
              <label htmlFor="translation-target">
                Translation target language
                {renderFieldStatus("translationTarget")}
              </label>
              <input
                id="translation-target"
                type="text"
                value={translationTarget}
                onChange={(e) => setTranslationTarget(e.target.value)}
                onBlur={handleTranslationTargetBlur}
                placeholder="e.g. English"
              />
              <p className="form-hint">
                Language to translate into when using the translation hotkey
                (Cmd+Shift+T). Common options: English, 中文, 日本語, 한국어,
                Français, Español, Deutsch. Saved automatically when you leave
                this field.
              </p>
            </div>

            {/* Personal dictionary section */}
            <div className="form-group">
              <label>Personal Dictionary</label>
              <p className="form-hint">
                Add brand names, technical jargon, or any terms that should be
                preserved verbatim during LLM polishing.
              </p>
              <div className="dictionary-input-row">
                <input
                  type="text"
                  value={newTerm}
                  onChange={(e) => setNewTerm(e.target.value)}
                  onKeyDown={handleTermKeyDown}
                  placeholder="e.g. OpenAI, Tauri, gRPC"
                />
                <button
                  type="button"
                  onClick={handleAddTerm}
                  className="primary"
                >
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
          </div>
        )}

        {/* Shortcuts tab — fixed shortcut list plus editable hotkey */}
        {activeTab === "shortcuts" && (
          <div className="tab-panel" role="tabpanel">
            <div className="shortcuts-list">
              {SHORTCUTS.map(({ label, keys }) => (
                <div key={label} className="shortcut-row">
                  <span className="shortcut-label">{label}</span>
                  <span className="shortcut-keys">
                    {keys.map((key, i) => (
                      <span key={key} style={{ display: "contents" }}>
                        {i > 0 && <span className="shortcut-plus">+</span>}
                        <kbd>{key}</kbd>
                      </span>
                    ))}
                  </span>
                </div>
              ))}
            </div>

            {/* Editable push-to-talk hotkey — requires explicit Apply + restart */}
            <div className="form-group" style={{ marginTop: "20px" }}>
              <label htmlFor="hotkey">Push-to-Talk Hotkey (custom)</label>
              <div className="dictionary-input-row">
                <input
                  id="hotkey"
                  type="text"
                  value={hotkey}
                  onChange={(e) => {
                    setHotkey(e.target.value);
                    setHotkeySaved(false);
                    setHotkeyError("");
                  }}
                  placeholder="e.g. CmdOrCtrl+Shift+Space"
                />
                <button
                  type="button"
                  className="primary"
                  onClick={handleSaveHotkey}
                  disabled={hotkeySaving}
                >
                  {hotkeySaving ? "Saving..." : "Apply"}
                </button>
              </div>
              {hotkeyError && <p className="error">{hotkeyError}</p>}
              {hotkeySaved && (
                <p
                  className="form-hint"
                  style={{ color: "var(--status-idle)" }}
                >
                  Hotkey saved. Changes take effect after restart.
                </p>
              )}
              {!hotkeyError && !hotkeySaved && (
                <p className="form-hint">
                  Type a combination (e.g. <code>CmdOrCtrl+Alt+Space</code>).
                  Takes effect after restart.
                </p>
              )}
            </div>

            <p className="shortcuts-note">
              Full shortcut customization coming soon
            </p>
          </div>
        )}

        {/* About tab */}
        {activeTab === "about" && (
          <div className="tab-panel" role="tabpanel">
            <div className="about-panel">
              <p className="about-app-name">Voice Typing</p>
              <p className="about-version">Version 0.1.0</p>

              <a
                href="https://github.com/huywang/voice-typing"
                target="_blank"
                className="about-link"
              >
                github.com/huywang/voice-typing
              </a>

              <div className="about-tech">
                <p>Tauri v2 + React + Rust</p>
                <p>ASR: Alibaba Cloud Paraformer</p>
                <p>LLM: Alibaba Cloud Qwen</p>
              </div>

              <p className="about-powered">
                Powered by Alibaba Cloud DashScope
              </p>
            </div>
          </div>
        )}
      </div>

      {/* Action row — only Back button remains; Save is now automatic */}
      <div className="actions">
        <button onClick={onBack}>Back</button>
      </div>
    </main>
  );
}

export default Settings;
