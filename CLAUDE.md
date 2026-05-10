# Voice Typing - AI Development Guide

## Project Overview

Cross-platform desktop voice typing application. Users press a global hotkey to record speech, which is sent to cloud AI services for recognition and text polishing, then injected into the currently focused input field.

## Tech Stack

- **Desktop Framework**: Tauri v2 (Rust backend + React frontend)
- **Frontend**: React 19 + TypeScript + Vite
- **Backend**: Rust
- **ASR**: Alibaba Cloud Paraformel (real-time streaming)
- **LLM**: Alibaba Cloud Qwen (text polishing)
- **Text Injection**: enigo crate (cross-platform keyboard simulation)
- **Audio Recording**: cpal crate (cross-platform audio I/O)
- **Package Manager**: pnpm

## Project Structure

```
voice-typing/
├── src/                    # Frontend (React + TypeScript)
│   ├── App.tsx
│   ├── main.tsx
│   ├── pages/              # Page components
│   ├── components/         # Reusable components
│   └── hooks/              # Custom hooks
├── src-tauri/              # Backend (Rust)
│   ├── src/
│   │   ├── lib.rs          # Tauri entry point
│   │   ├── main.rs         # Binary entry
│   │   ├── commands.rs     # Tauri IPC commands
│   │   ├── audio/          # Audio recording module
│   │   ├── asr/            # Cloud ASR client
│   │   ├── llm/            # Cloud LLM client
│   │   └── injector/       # Text injection module
│   ├── Cargo.toml
│   └── tauri.conf.json
├── .github/
│   ├── workflows/          # CI/CD pipelines
│   └── ISSUE_TEMPLATE/     # Issue templates
└── CLAUDE.md               # This file
```

## Development Commands

```bash
pnpm install          # Install dependencies
pnpm tauri dev        # Run in development mode
pnpm tauri build      # Build for production
pnpm build            # Build frontend only
cargo test --manifest-path src-tauri/Cargo.toml  # Run Rust tests
cargo clippy --manifest-path src-tauri/Cargo.toml -- -D warnings  # Lint Rust code
pnpm tsc --noEmit     # Type-check frontend
```

## Coding Standards

### Rust (Backend)
- Follow Rust 2021 edition conventions
- All public functions must have doc comments
- Use `thiserror` for custom error types
- Use `tokio` for async runtime
- Run `cargo clippy -- -D warnings` before committing
- Run `cargo test` before committing
- Keep modules focused (single responsibility)

### TypeScript (Frontend)
- Strict TypeScript (no `any` unless absolutely necessary)
- Functional components with hooks
- Use `@tauri-apps/api` for IPC with Rust backend
- Comments in English to match codebase

### General
- Commit messages: conventional commits format (feat/fix/chore/docs/refactor/test)
- One feature per PR, keep PRs small and focused
- All new features must include unit tests
- Error handling: never silently swallow errors

## Architecture Decisions

1. **Cloud-only AI**: No local models. ASR and LLM are both cloud services (Alibaba Cloud). Requires internet connection.
2. **Text injection via enigo**: Simulates keyboard input to inject text into any focused input field. Fallback to clipboard paste if direct injection fails.
3. **Push-to-Talk**: Global hotkey triggers recording. Release to stop and process.
4. **System tray app**: Minimal UI, lives in system tray. Small overlay window shows recording status.
5. **Platform support**: macOS (primary), Windows, Linux (X11 only for MVP).

## API Keys

API keys are stored in local config file managed by `tauri-plugin-store`. Never hardcode API keys. The app should guide users to configure their own Alibaba Cloud API keys on first run.

## Testing Strategy

- **Rust unit tests**: Core logic (audio processing, API client parsing, text injection logic)
- **Integration tests**: End-to-end flow with mocked API responses
- **CI checks**: `cargo check`, `cargo test`, `cargo clippy`, `tsc --noEmit`
