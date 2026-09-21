/**
 * Tailwind utility bundles shared across the overlay + prefs views —
 * the replacement for the retired `.mv-*` / `.prf-*` sheet (DESIGN.md
 * §6). `cn` (re-exported from @marvis/ui) joins conditional fragments
 * and dedupes conflicting classes so per-instance overrides win; the
 * consts keep repeated control patterns in one place so a token change
 * stays one edit.
 */
import { cn } from '@marvis/ui';

export { cn };

/* ─── text atoms ────────────────────────────────────────────────── */
export const NUM = 'font-mono tabular-nums';
export const META = 'font-mono text-[12.5px] text-muted-foreground';
export const EMPTY = 'text-[12.5px] text-muted-foreground';
export const KBD =
  'inline-flex min-w-5.5 items-center justify-center rounded-[5px] border border-b-2 border-border bg-fg-soft px-1.5 py-0.5 font-mono text-[11px] text-fg-2';

/* ─── buttons (spec §6) ─────────────────────────────────────────── */
/* Overlay controls are 26px (BTN_SM); the decorated prefs window steps
   up to 30px (BTN_LG). Pair a size with a variant: `cn(BTN_LG,
   BTN_PRIMARY)`. Weight lives in the size/link consts so nothing
   double-declares a property. */
const BTN_BASE =
  'inline-flex items-center justify-center gap-1.5 rounded-full border whitespace-nowrap transition-[background,transform,border-color,color] duration-(--motion-fast) ease-(--ease) enabled:active:scale-[0.97] disabled:cursor-default disabled:opacity-50 motion-reduce:transition-none';
export const BTN_SM = `${BTN_BASE} h-6.5 px-3 text-[11.5px] font-semibold`;
export const BTN_LG = `${BTN_BASE} h-7.5 px-3.5 text-[12.5px] font-semibold`;
export const BTN_LINK_SM = `${BTN_BASE} h-auto border-transparent px-1.5 text-[11.5px] font-medium`;
export const BTN_LINK_LG = `${BTN_BASE} h-auto border-transparent px-1.5 text-[12.5px] font-medium`;
export const BTN_PRIMARY =
  'border-transparent bg-primary text-primary-foreground enabled:hover:bg-[color-mix(in_oklch,var(--primary)_88%,black_12%)] enabled:active:bg-[color-mix(in_oklch,var(--primary)_74%,black_26%)]';
export const BTN_OUTLINE =
  'border-border bg-transparent text-foreground enabled:hover:border-[color-mix(in_oklch,var(--fg)_30%,var(--border))]';
export const BTN_LINK =
  'bg-transparent text-muted-foreground enabled:hover:text-accent-text enabled:hover:underline';
export const BTN_DANGER =
  'enabled:hover:border-[color-mix(in_oklch,var(--destructive)_55%,var(--border))] enabled:hover:text-destructive';

/* 26px round ghost button (bar + panels). Icons are sized at the
   markup site (`size-*` on the icon itself). */
export const ICON_BTN =
  'grid size-6.5 flex-none cursor-pointer place-items-center rounded-full border-0 bg-transparent text-muted-foreground transition-[background,color] duration-(--motion-fast) ease-(--ease) enabled:hover:bg-fg-soft enabled:hover:text-foreground disabled:cursor-not-allowed disabled:opacity-45 motion-reduce:transition-none';

/* ─── fields (prefs window only — the 30px control size) ────────── */
export const FIELD =
  'h-7.5 min-w-0 flex-1 rounded-lg border border-border bg-input-well px-2.5 font-mono text-xs text-foreground outline-none transition-[border-color,box-shadow] duration-(--motion-fast) ease-(--ease) placeholder:font-sans placeholder:text-muted-foreground focus:border-accent focus:shadow-(--focus-ring) motion-reduce:transition-none';
export const MODEL_SEL =
  'h-7.5 w-full min-w-0 flex-1 rounded-lg border border-border bg-input-well px-2 font-mono text-[11.5px] text-foreground outline-none focus:border-accent focus:shadow-(--focus-ring) disabled:opacity-50';
export const LBL = 'flex-none text-[11.5px] text-muted-foreground';
export const PROV_NOTE =
  'mt-1.25 text-[11px] leading-[1.35] break-words text-muted-foreground';
export const PROV_ERR =
  'mt-1.25 text-[11px] leading-[1.35] break-words text-destructive';

/* ─── panel + toast chrome ──────────────────────────────────────── */
export const PANEL =
  'flex flex-col overflow-hidden rounded-[18px] border border-border bg-[color-mix(in_oklch,var(--surface)_90%,transparent)] backdrop-blur-lg';
export const PANEL_HEAD =
  'flex items-start gap-1.5 border-b border-border px-3 py-2.25';
export const PANEL_BODY =
  'min-h-0 flex-1 select-text overflow-y-auto px-3.5 pt-3 pb-3.5 text-[13px] leading-[1.6]';
export const CHIP =
  'inline-flex items-center gap-1.25 rounded-full border border-border bg-[color-mix(in_oklch,var(--surface)_60%,transparent)] px-2 py-0.75 font-mono text-[10.5px] text-muted-foreground';
export const SPIN =
  'size-3 animate-rotate rounded-full border-2 border-[color-mix(in_oklch,var(--muted)_30%,transparent)] border-t-foreground motion-reduce:animate-none';

/* ─── prefs rows ────────────────────────────────────────────────── */
export const PROV_CARD =
  'mb-2.5 rounded-xl border bg-surface px-3.5 py-3 transition-[border-color,opacity] duration-(--motion-fast) ease-(--ease) motion-reduce:transition-none';
export const PRF_ROWS = 'border-t border-border';
export const PRF_ROW =
  'flex items-center justify-between gap-4 border-b border-border py-3 first-of-type:pt-0.5';
export const PR_LABEL = 'text-[13px] font-[550]';
export const PR_SUB = 'mt-0.5 max-w-[40ch] text-[11.5px] text-muted-foreground';
export const PR_CTL = 'inline-flex flex-none items-center gap-2';
export const H2 = 'mb-1 text-[17px] font-[650] tracking-[-0.01em]';
export const SUB = 'mb-4 text-[12.5px] leading-[1.5] text-muted-foreground';

/* react-markdown output can't take classNames, so the sheet styles
   every element as a descendant of this wrapper (was `.ask-md`). */
export const ASK_MD = cn(
  'leading-[1.5]',
  '[&_h1]:mt-[0.6em] [&_h1]:mb-[0.3em] [&_h1]:text-[1.05rem] [&_h1]:font-semibold',
  '[&_h2]:mt-[0.6em] [&_h2]:mb-[0.3em] [&_h2]:text-[1rem] [&_h2]:font-semibold',
  '[&_h3]:mt-[0.6em] [&_h3]:mb-[0.3em] [&_h3]:text-[0.92rem] [&_h3]:font-semibold',
  '[&_h4]:mt-[0.6em] [&_h4]:mb-[0.3em] [&_h4]:text-[0.92rem] [&_h4]:font-semibold',
  '[&_p]:my-[0.45em]',
  '[&_ul]:my-[0.45em] [&_ul]:list-disc [&_ul]:pl-[1.4em]',
  '[&_ol]:my-[0.45em] [&_ol]:list-decimal [&_ol]:pl-[1.4em]',
  '[&_li]:my-[0.15em] [&_li>p]:my-[0.15em]',
  '[&_code]:rounded [&_code]:bg-fg-soft [&_code]:px-[0.3em] [&_code]:py-[0.1em] [&_code]:font-mono [&_code]:text-[0.82em]',
  '[&_pre]:my-2 [&_pre]:overflow-x-auto [&_pre]:rounded-lg [&_pre]:bg-fg-soft [&_pre]:px-[0.75em] [&_pre]:py-[0.55em]',
  '[&_pre_code]:bg-transparent [&_pre_code]:p-0',
  '[&_a]:text-accent-text [&_a]:underline',
  '[&_blockquote]:my-2 [&_blockquote]:border-l-3 [&_blockquote]:border-border [&_blockquote]:pl-3 [&_blockquote]:text-muted-foreground',
  '[&_table]:my-2 [&_table]:border-collapse',
  '[&_th]:border [&_th]:border-border [&_th]:px-[0.6em] [&_th]:py-1',
  '[&_td]:border [&_td]:border-border [&_td]:px-[0.6em] [&_td]:py-1',
  '[&_hr]:my-[0.6em] [&_hr]:border-border',
  '[&>:first-child]:mt-0 [&>:last-child]:mb-0',
);
