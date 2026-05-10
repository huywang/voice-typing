import { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";

// Permission status returned by the backend check_permissions command.
interface PermissionStatus {
  microphone: boolean;
  accessibility: boolean;
}

interface OnboardingProps {
  /** Called when the user finishes the onboarding flow. */
  onComplete: () => void;
}

// Total number of wizard steps (0-indexed internally, shown as 1-based).
const TOTAL_STEPS = 5;

// ---- Individual step components ------------------------------------------------

function StepWelcome() {
  return (
    <div className="onboarding-step">
      <div className="onboarding-icon">🎙️</div>
      <h2>Welcome to Voice Typing</h2>
      <p>
        Voice Typing lets you dictate text into any app using a global hotkey.
        Speech is transcribed by Alibaba Cloud's Paraformer ASR and optionally
        polished by Qwen LLM.
      </p>
      <ul className="onboarding-feature-list">
        <li>Hold the hotkey to record, release to transcribe</li>
        <li>Works in any text field across all apps</li>
        <li>Optional AI text polishing (remove fillers, fix punctuation)</li>
      </ul>
    </div>
  );
}

interface StepPermissionsProps {
  permissions: PermissionStatus;
  onOpenSettings: (pane: string) => void;
}

function StepPermissions({ permissions, onOpenSettings }: StepPermissionsProps) {
  return (
    <div className="onboarding-step">
      <div className="onboarding-icon">🔐</div>
      <h2>Required Permissions</h2>
      <p>
        Voice Typing needs two macOS permissions to function. Grant them below,
        then continue.
      </p>

      <div className="onboarding-permission-card">
        <div className="permission-title">🎤 Microphone Access</div>
        <div className="permission-desc">
          Needed to capture your speech.
        </div>
        <div className="permission-status-row">
          <span
            className={`permission-dot ${permissions.microphone ? "permission-dot--granted" : "permission-dot--denied"}`}
            aria-label={permissions.microphone ? "Granted" : "Not granted"}
          />
          <span className={`permission-badge ${permissions.microphone ? "permission-badge--granted" : "permission-badge--denied"}`}>
            {permissions.microphone ? "Granted" : "Not Granted"}
          </span>
          {!permissions.microphone && (
            <button
              type="button"
              className="permission-open-btn"
              onClick={() => onOpenSettings("microphone")}
            >
              Open Settings
            </button>
          )}
        </div>
      </div>

      <div className="onboarding-permission-card">
        <div className="permission-title">♿ Accessibility Access</div>
        <div className="permission-desc">
          Needed to inject typed text into other apps.
        </div>
        <div className="permission-status-row">
          <span
            className={`permission-dot ${permissions.accessibility ? "permission-dot--granted" : "permission-dot--denied"}`}
            aria-label={permissions.accessibility ? "Granted" : "Not granted"}
          />
          <span className={`permission-badge ${permissions.accessibility ? "permission-badge--granted" : "permission-badge--denied"}`}>
            {permissions.accessibility ? "Granted" : "Not Granted"}
          </span>
          {!permissions.accessibility && (
            <button
              type="button"
              className="permission-open-btn"
              onClick={() => onOpenSettings("accessibility")}
            >
              Open Settings
            </button>
          )}
        </div>
      </div>

      <p className="form-hint" style={{ marginTop: 8 }}>
        Status refreshes automatically every 5 seconds.
      </p>
    </div>
  );
}

interface StepApiKeyProps {
  apiKey: string;
  setApiKey: (v: string) => void;
  llmEnabled: boolean;
  setLlmEnabled: (v: boolean) => void;
  saving: boolean;
  error: string;
}

function StepApiKey({
  apiKey,
  setApiKey,
  llmEnabled,
  setLlmEnabled,
  saving,
  error,
}: StepApiKeyProps) {
  return (
    <div className="onboarding-step settings">
      <div className="onboarding-icon">🔑</div>
      <h2>API Key Setup</h2>
      <p>
        Enter your Alibaba Cloud DashScope API key. This key is stored locally
        on your device and never sent anywhere other than Alibaba Cloud.
      </p>

      <div className="form-group">
        <label htmlFor="ob-api-key">DashScope API Key</label>
        <input
          id="ob-api-key"
          type="password"
          value={apiKey}
          onChange={(e) => setApiKey(e.target.value)}
          placeholder="sk-..."
          disabled={saving}
        />
        <p className="form-hint">
          Get your key from{" "}
          <a href="https://dashscope.console.aliyun.com/" target="_blank" rel="noreferrer">
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
            disabled={saving}
          />
          Enable LLM text polishing (Qwen)
        </label>
        <p className="form-hint">
          Polishes transcribed text to remove filler words and fix punctuation.
        </p>
      </div>

      {error && <p className="error">{error}</p>}
    </div>
  );
}

function StepHotkey() {
  return (
    <div className="onboarding-step">
      <div className="onboarding-icon">⌨️</div>
      <h2>Your Hotkey</h2>
      <p>
        Use the global hotkey to start and stop recording from any app — no
        need to switch to Voice Typing.
      </p>

      <div className="onboarding-hotkey-demo">
        <div className="hotkey-label">Push-to-Talk</div>
        <div className="hotkey-keys">
          <kbd>⌘</kbd>
          <span className="hotkey-plus">+</span>
          <kbd>⇧</kbd>
          <span className="hotkey-plus">+</span>
          <kbd>Space</kbd>
        </div>
        <div className="hotkey-hint">
          Hold to record · Release to transcribe and inject text
        </div>
      </div>

      <div className="onboarding-hotkey-demo" style={{ marginTop: 12 }}>
        <div className="hotkey-label">Re-inject Last Text</div>
        <div className="hotkey-keys">
          <kbd>⌘</kbd>
          <span className="hotkey-plus">+</span>
          <kbd>⌥</kbd>
          <span className="hotkey-plus">+</span>
          <kbd>V</kbd>
        </div>
        <div className="hotkey-hint">Paste the last transcription again</div>
      </div>
    </div>
  );
}

function StepDone() {
  return (
    <div className="onboarding-step">
      <div className="onboarding-icon">🎉</div>
      <h2>You're All Set!</h2>
      <p>
        Voice Typing is ready to use. Press{" "}
        <kbd>Cmd+Shift+Space</kbd> in any app to start dictating.
      </p>
      <p className="onboarding-tip">
        The app lives in your menu bar. Click the icon any time to open
        settings, view history, or quit.
      </p>
    </div>
  );
}

// ---- Main wizard component -----------------------------------------------------

function Onboarding({ onComplete }: OnboardingProps) {
  const [step, setStep] = useState(0);

  // API key form state (used on step 2)
  const [apiKey, setApiKey] = useState("");
  const [llmEnabled, setLlmEnabled] = useState(true);
  const [saving, setSaving] = useState(false);
  const [apiError, setApiError] = useState("");

  // Permission status — polled every 5 seconds while on the permissions step.
  const [permissions, setPermissions] = useState<PermissionStatus>({
    microphone: false,
    accessibility: false,
  });

  useEffect(() => {
    const fetchPermissions = () => {
      invoke<PermissionStatus>("check_permissions")
        .then((status) => setPermissions(status))
        .catch(() => {
          // Non-fatal: keep defaults.
        });
    };

    fetchPermissions();
    const interval = setInterval(fetchPermissions, 5000);
    return () => clearInterval(interval);
  }, []);

  // Open a macOS System Settings pane for the given permission.
  const handleOpenSettings = async (pane: string) => {
    try {
      await invoke("open_system_preferences", { pane });
    } catch (e) {
      console.error("Failed to open system preferences:", e);
    }
  };

  const isLastStep = step === TOTAL_STEPS - 1;
  const isApiStep = step === 2;

  /** Save API config and advance to the next step. */
  const handleNextOnApiStep = async () => {
    if (!apiKey.trim()) {
      setApiError("Please enter your DashScope API key");
      return;
    }
    setSaving(true);
    setApiError("");
    try {
      await invoke("set_api_config", {
        apiKey: apiKey.trim(),
        llmEnabled,
      });
      setStep((s) => s + 1);
    } catch (e) {
      setApiError(`Failed to save: ${e}`);
    } finally {
      setSaving(false);
    }
  };

  /** Complete onboarding: persist the flag and notify the parent. */
  const handleFinish = async () => {
    try {
      await invoke("complete_onboarding");
    } catch (e) {
      console.error("Failed to complete onboarding:", e);
    }
    onComplete();
  };

  const handleNext = async () => {
    if (isLastStep) {
      await handleFinish();
    } else if (isApiStep) {
      await handleNextOnApiStep();
    } else {
      setStep((s) => s + 1);
    }
  };

  const handleBack = () => {
    if (step > 0) setStep((s) => s - 1);
  };

  const renderStep = () => {
    switch (step) {
      case 0:
        return <StepWelcome />;
      case 1:
        return (
          <StepPermissions
            permissions={permissions}
            onOpenSettings={handleOpenSettings}
          />
        );
      case 2:
        return (
          <StepApiKey
            apiKey={apiKey}
            setApiKey={setApiKey}
            llmEnabled={llmEnabled}
            setLlmEnabled={setLlmEnabled}
            saving={saving}
            error={apiError}
          />
        );
      case 3:
        return <StepHotkey />;
      case 4:
        return <StepDone />;
      default:
        return null;
    }
  };

  return (
    <main className="container onboarding">
      {/* Progress dots */}
      <div className="onboarding-progress">
        {Array.from({ length: TOTAL_STEPS }).map((_, i) => (
          <div
            key={i}
            className={`onboarding-dot ${i === step ? "active" : i < step ? "done" : ""}`}
          />
        ))}
      </div>

      {/* Step content */}
      {renderStep()}

      {/* Navigation */}
      <div className="onboarding-nav">
        <button onClick={handleBack} disabled={step === 0 || saving}>
          Back
        </button>
        <button className="primary" onClick={handleNext} disabled={saving}>
          {saving ? "Saving..." : isLastStep ? "Start Using" : "Next"}
        </button>
      </div>
    </main>
  );
}

export default Onboarding;
