# MVP Issue Backlog (v0.1.0)

Push to GitHub after initial commit, then create these as Issues with `ai-task` label.

---

## Issue #1: Audio recording module

**Title**: feat: implement audio recording module with cpal

**Description**:
Implement a cross-platform audio recording module using the `cpal` crate.

Requirements:
- Record from default microphone input
- Output format: 16kHz, mono, f32 PCM samples
- Provide start/stop recording API
- Buffer recorded audio in memory
- Export recorded audio as WAV bytes for ASR consumption

**Acceptance Criteria**:
- [ ] `AudioRecorder` struct with `start()`, `stop()`, `get_wav_data()` methods
- [ ] Records from system default input device
- [ ] Output sample rate is 16kHz mono
- [ ] Unit tests for WAV encoding
- [ ] `cpal` added to Cargo.toml dependencies
- [ ] Module at `src-tauri/src/audio/`

**Files**:
- `src-tauri/src/audio/mod.rs` (create)
- `src-tauri/src/audio/recorder.rs` (create)
- `src-tauri/src/audio/wav.rs` (create)
- `src-tauri/src/lib.rs` (modify - add audio module)
- `src-tauri/Cargo.toml` (modify - add cpal, hound)

---

## Issue #2: Alibaba Cloud ASR client

**Title**: feat: implement Alibaba Cloud ASR client

**Description**:
Implement a client for Alibaba Cloud's Paraformer speech recognition service.

Requirements:
- Send WAV audio data to Alibaba Cloud ASR API
- Parse and return recognized text
- Support Chinese and English recognition
- Handle API errors gracefully
- API key read from tauri-plugin-store config

**Acceptance Criteria**:
- [ ] `AsrClient` struct with `recognize(wav_data: &[u8]) -> Result<String>`
- [ ] Correct API request format per Alibaba Cloud docs
- [ ] Error handling for network failures and API errors
- [ ] Unit tests with mocked HTTP responses
- [ ] Module at `src-tauri/src/asr/`

**Files**:
- `src-tauri/src/asr/mod.rs` (create)
- `src-tauri/src/asr/client.rs` (create)
- `src-tauri/src/lib.rs` (modify)
- `src-tauri/Cargo.toml` (modify - add reqwest, tokio)

---

## Issue #3: Alibaba Cloud LLM client for text polishing

**Title**: feat: implement LLM client for text polishing

**Description**:
Implement a client for Alibaba Cloud Qwen LLM to polish recognized text.

Requirements:
- Send raw ASR text to Qwen API
- System prompt: clean up filler words, fix punctuation, maintain original meaning
- Return polished text
- Make polishing optional (can be toggled off)
- API key read from tauri-plugin-store config

**Acceptance Criteria**:
- [ ] `LlmClient` struct with `polish(raw_text: &str) -> Result<String>`
- [ ] Appropriate system prompt for text polishing
- [ ] Bypass option that returns raw text unchanged
- [ ] Error handling with fallback to raw text
- [ ] Unit tests with mocked responses
- [ ] Module at `src-tauri/src/llm/`

**Files**:
- `src-tauri/src/llm/mod.rs` (create)
- `src-tauri/src/llm/client.rs` (create)
- `src-tauri/src/lib.rs` (modify)
- `src-tauri/Cargo.toml` (modify)

---

## Issue #4: Text injection module with enigo

**Title**: feat: implement text injection via enigo

**Description**:
Implement cross-platform text injection using the `enigo` crate to simulate keyboard input.

Requirements:
- Inject arbitrary Unicode text into the currently focused input field
- Support macOS, Windows, Linux (X11)
- Handle errors gracefully (e.g., permission denied on macOS)

**Acceptance Criteria**:
- [ ] `TextInjector` struct with `inject(text: &str) -> Result<()>`
- [ ] Works on macOS (requires Accessibility permission)
- [ ] Works on Windows
- [ ] Works on Linux X11
- [ ] Error types for permission issues
- [ ] Module at `src-tauri/src/injector/`

**Files**:
- `src-tauri/src/injector/mod.rs` (create)
- `src-tauri/src/injector/text_input.rs` (create)
- `src-tauri/src/lib.rs` (modify)
- `src-tauri/Cargo.toml` (modify - add enigo)

---

## Issue #5: Global hotkey and core pipeline

**Title**: feat: wire up global hotkey with recording-recognition-injection pipeline

**Description**:
Connect all modules into the core voice typing pipeline, triggered by a global hotkey.

Flow: Hold hotkey → start recording → release hotkey → stop recording → ASR → LLM polish → inject text

Requirements:
- Register global hotkey (default: Ctrl+Shift+Space / Cmd+Shift+Space)
- Push-to-talk: hold to record, release to process
- Coordinate async pipeline: record → ASR → LLM → inject
- Expose Tauri IPC commands for frontend status updates
- Handle errors at each stage with user-visible feedback

**Acceptance Criteria**:
- [ ] Global hotkey registered and working
- [ ] Full pipeline: hotkey → record → ASR → LLM → inject
- [ ] Tauri commands: `get_status`, `set_api_keys`
- [ ] Error propagation to frontend
- [ ] Works on macOS (primary test platform)

**Files**:
- `src-tauri/src/commands.rs` (create)
- `src-tauri/src/pipeline.rs` (create)
- `src-tauri/src/lib.rs` (modify - register commands and plugins)
- `src-tauri/Cargo.toml` (modify - add tauri-plugin-global-shortcut, tauri-plugin-store)

---

## Issue #6: Settings page and API key configuration

**Title**: feat: add settings page for API key configuration

**Description**:
Create a React settings page where users can configure their Alibaba Cloud API keys and basic preferences.

Requirements:
- Input fields for Alibaba Cloud AccessKey ID and Secret
- Toggle for LLM polishing on/off
- Hotkey configuration
- Persist settings via tauri-plugin-store
- First-run detection: show settings if keys not configured

**Acceptance Criteria**:
- [ ] Settings page with form inputs
- [ ] API keys saved securely via tauri-plugin-store
- [ ] Settings persisted across app restarts
- [ ] First-run redirect to settings page
- [ ] Basic form validation

**Files**:
- `src/pages/Settings.tsx` (create)
- `src/App.tsx` (modify - add routing)
- `src-tauri/src/commands.rs` (modify - add settings commands)

---

## Issue #7: System tray with status indicator

**Title**: feat: add system tray with status indicator

**Description**:
Implement system tray icon that shows the app status and provides quick actions.

Requirements:
- Tray icon with different states: idle, recording, processing
- Right-click menu: Settings, Quit
- Click to show/hide main window
- Status changes reflected in icon appearance

**Acceptance Criteria**:
- [ ] System tray icon visible
- [ ] Icon changes based on app state
- [ ] Context menu with Settings and Quit
- [ ] Click toggles main window
- [ ] Works on macOS and Windows

**Files**:
- `src-tauri/src/tray.rs` (create)
- `src-tauri/src/lib.rs` (modify)
- `src-tauri/icons/` (add status icons)

---

## Issue #8: Recording status overlay

**Title**: feat: add recording status overlay window

**Description**:
Show a small floating overlay when recording is active, indicating that the app is listening.

Requirements:
- Small transparent overlay window
- Shows "Recording..." with audio level indicator
- Appears near cursor or center of screen
- Auto-hides when recording stops

**Acceptance Criteria**:
- [ ] Overlay appears when recording starts
- [ ] Shows audio level visualization
- [ ] Disappears when recording stops
- [ ] Always on top, click-through
- [ ] Minimal and non-intrusive design

**Files**:
- `src/components/RecordingOverlay.tsx` (create)
- `src-tauri/tauri.conf.json` (modify - add overlay window config)
- `src-tauri/src/lib.rs` (modify - manage overlay window)

---

## Suggested order

1. Issue #1 (Audio) - no dependencies
2. Issue #4 (Text injection) - no dependencies
3. Issue #2 (ASR) - no dependencies
4. Issue #3 (LLM) - no dependencies
5. Issue #5 (Pipeline) - depends on #1, #2, #3, #4
6. Issue #6 (Settings) - can parallel with #5
7. Issue #7 (Tray) - after #5
8. Issue #8 (Overlay) - after #5
