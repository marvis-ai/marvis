/**
 * The six `app.main_language` codes — validated server-side by
 * `config::validate_main_language`, so every picker surface (Settings →
 * General, the onboarding step) shares this list.
 */
export const LANGUAGES = [
  { id: 'en', label: 'English' },
  { id: 'zh', label: '中文' },
  { id: 'ja', label: '日本語' },
  { id: 'ko', label: '한국어' },
  { id: 'fr', label: 'Français' },
  { id: 'es', label: 'Español' },
] as const;

/** English gets local Whisper; every other language (CJK especially)
 * gets Sherpa's multilingual SenseVoice. Onboarding seeds
 * `models.stt_provider` with this — the voice step can still override. */
export const recommendedSttProvider = (mainLanguage: string) =>
  mainLanguage === 'en' ? 'whisper' : 'sherpa';
