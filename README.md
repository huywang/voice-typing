# Voice Typing

Cross-platform desktop voice typing tool. Press a hotkey, speak, and text appears in any input field.

Powered by Alibaba Cloud ASR (Paraformer) + Qwen LLM for recognition and text polishing.

## Features (Planned)

- Global push-to-talk hotkey
- Real-time speech recognition via Alibaba Cloud
- LLM-powered text polishing (remove filler words, fix grammar)
- Text injection into any focused input field
- System tray with status indicator
- Cross-platform: macOS, Windows, Linux (X11)

## Development

```bash
pnpm install
pnpm tauri dev
```

## Architecture

```
Microphone → Cloud ASR (Alibaba Paraformer) → Raw Text → Cloud LLM (Qwen) → Polished Text → enigo (keyboard simulation) → Input Field
```

## Tech Stack

- [Tauri v2](https://v2.tauri.app/) - Desktop framework
- React 19 + TypeScript - Frontend
- Rust - Backend
- Alibaba Cloud - ASR & LLM services

## License

MIT
