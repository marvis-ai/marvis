export interface FaqItem {
  question: string;
  answer: string;
}

export const FAQS: FaqItem[] = [
  {
    question: 'What is Marvis AI?',
    answer:
      'Marvis is a free, open-source personal AI copilot for macOS, Windows, and Linux. It sits in a subtle floating bar above your workspace, understands your screen with explicit permission, transcribes meetings locally, and connects directly to your chosen AI models with zero cloud storage.',
  },
  {
    question: 'How does Marvis protect user privacy compared to other AI assistants?',
    answer:
      'Marvis uses a local-first architecture. There are no user accounts, no telemetry servers, and no cloud synchronization. Your API keys are stored locally at 0600 permissions in ~/.marvis/keys.json and masked in the UI. Screen frames are kept only in a rolling 60-second in-memory ring (never saved to disk), and meeting transcription runs on-device using whisper.cpp and sherpa-onnx.',
  },
  {
    question: 'Can Marvis run completely offline with local models?',
    answer:
      'Yes. You can connect Marvis to local LLMs via Ollama or any OpenAI-compatible local server. Speech recognition runs fully offline using bundled whisper.cpp or sherpa models, enabling 100% air-gapped personal AI assistance on your machine.',
  },
  {
    question: 'Which operating systems and hardware are supported?',
    answer:
      'Marvis runs natively on macOS (Apple Silicon and Intel via ScreenCaptureKit), Windows (x86_64 via Windows Graphics Capture and WASAPI audio), and Linux (x86_64 via XDG desktop portal and PipeWire).',
  },
  {
    question: 'Which AI providers and speech engines can I use?',
    answer:
      'Marvis supports Bring Your Own Key (BYOK) for OpenAI, Anthropic Claude, Google Gemini, OpenRouter, and Ollama (local). For speech transcription, it supports on-device whisper.cpp, sherpa ASR, and optional streaming Deepgram.',
  },
  {
    question: 'Is Marvis really free and open source?',
    answer:
      'Yes, Marvis is 100% free and open-source software licensed under the Apache 2.0 license. The complete source code, build scripts, and desktop application releases are available publicly on GitHub.',
  },
];
