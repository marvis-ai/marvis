/**
 * Composer preset helpers — the `/name` shorthand's token matcher, the
 * `{input}`/`{lang}` expansion for armed presets, and the derived
 * "template" read (`{input}` in the text expands into the message).
 * Mirrors `src-tauri/src/presets.rs` (catalog order is the same list
 * the backend serves via `presets_list`).
 */
import type { Preset } from './commands';

/** `{input}` in the text is the whole contract — with it, the preset
 *  expands into the sent message at send time; without it, the text is
 *  a silent system-prompt instruction. Mirrors `presets::is_template`. */
export const isTemplate = (p: Preset): boolean => p.text.includes('{input}');

/** Whether the preset takes a `{lang}` param — the armed composer
 *  shows its editable language badge for these. */
export const hasLangParam = (p: Preset): boolean => p.text.includes('{lang}');

/** The caret-0 `/token`: `text` starts with `/`; the token is the
 *  `[a-z0-9-]` run after it — the `i` flag admits A–Z too, so `token`
 *  keeps its typed case (`/Sum` → `Sum`; `matchPreset` lowercases for
 *  the compare). `rest` is everything that follows (leading
 *  whitespace included). `null` when text isn't a slash command. */
export const slashToken = (
  text: string,
): { token: string; rest: string } | null => {
  const m = /^\/([a-z0-9-]*)([\s\S]*)$/i.exec(text);
  if (!m) return null;
  return { token: m[1], rest: m[2] };
};

/** A name's `/`-token form — lowercase, each non-`[a-z0-9]` run
 *  collapses to one `-`, edges trimmed (`Reply nicely` →
 *  `reply-nicely`, `Devil's advocate` → `devil-s-advocate`). Customs
 *  have no memorable id suffix, so this keeps spaced/punctuated names
 *  typeable inside the token charset. */
export const presetToken = (name: string): string =>
  name
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-+|-+$/g, '');

/** A `/token`'s name part for the palette's filter seed — `null`
 *  when `text` isn't slash-prefixed. `/trans hello` → `trans`. */
export const slashQuery = (text: string): string | null =>
  slashToken(text)?.token ?? null;

/** Drop a leading `/token` plus ONE following space — the pick path
 *  consumes the trigger text: `/trans hello` → `hello`, `/` → ``.
 *  A non-slash text passes through unchanged. */
export const stripSlashToken = (text: string): string => {
  const s = slashToken(text);
  if (!s) return text;
  return s.rest.startsWith(' ') ? s.rest.slice(1) : s.rest;
};

/** First preset matching `token` — full id, id suffix (`b:sum` →
 *  `sum`), name, or the name's `presetToken` slug, all
 *  case-insensitive; list order is the catalog's (built-ins then
 *  customs), so a custom never shadows a built-in. */
export const matchPreset = (
  token: string,
  presets: Preset[],
): Preset | null => {
  const t = token.trim().toLowerCase();
  if (!t) return null;
  return (
    presets.find(
      (p) =>
        p.id.toLowerCase() === t ||
        p.id.split(':')[1]?.toLowerCase() === t ||
        p.name.toLowerCase() === t ||
        presetToken(p.name) === t,
    ) ?? null
  );
};

export interface SlashHit {
  preset: Preset;
  /** Composer text after the consumed `/token` (+ one space). */
  rest: string;
}

/** Leading-`/token` resolution. The eager pass (per keystroke,
 *  `endOfText: false`) requires a whitespace terminator — an
 *  end-of-text token stays literal so a shorter prefix name can't
 *  fire mid-word (`/sum` while `summarize` also exists). The send-time
 *  pass (`endOfText: true`) additionally accepts the bare end-of-text
 *  token. */
export const resolveSlash = (
  text: string,
  presets: Preset[],
  endOfText: boolean,
): SlashHit | null => {
  const slash = slashToken(text);
  if (!slash) return null;
  const terminated = /^\s/.test(slash.rest);
  if (!terminated && !(endOfText && slash.rest === '')) return null;
  const preset = matchPreset(slash.token, presets);
  if (!preset) return null;
  return { preset, rest: terminated ? slash.rest.replace(/^\s/, '') : '' };
};

/** English names mirroring `prompts::language_name` — `{lang}` expands
 *  at pick time so the composer shows the concrete instruction. A Map,
 *  not a plain object: `Record` lookup leaks prototype keys
 *  (`langName('toString')` would return a function). */
const LANG_NAMES = new Map<string, string>([
  ['zh', 'Chinese'],
  ['ja', 'Japanese'],
  ['ko', 'Korean'],
  ['fr', 'French'],
  ['es', 'Spanish'],
]);

export const langName = (code: string): string =>
  LANG_NAMES.get(code.trim()) ?? 'English';

/** The palette's whole keyboard contract — the set the composer
 *  forwards (`AskInput`) while a `/`-session is up AND the palette
 *  window itself listens for when it holds key focus. One shared list
 *  so the two focus modes can't drift. */
export const PALETTE_KEYS: readonly string[] = [
  'Escape',
  'ArrowDown',
  'ArrowUp',
  'Home',
  'End',
  'Enter',
  'Tab',
];

/** `{lang}`/`{input}` substitution. `{input}` absent → `input`
 *  appended after the template; empty input clears the placeholder so
 *  the caret lands where the argument goes. */
export const expandTemplate = (
  text: string,
  input: string,
  lang: string,
): string => {
  // split/join rather than replaceAll — the app targets ES2020, and the
  // literal insertion never interprets `$` patterns in user input.
  const withLang = text.split('{lang}').join(lang);
  if (withLang.includes('{input}')) {
    return withLang.split('{input}').join(input);
  }
  return input ? `${withLang}\n\n${input}` : withLang;
};
