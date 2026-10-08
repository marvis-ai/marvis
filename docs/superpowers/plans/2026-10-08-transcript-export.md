# Transcript Export Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Export a Listen session as a `.md` file via a native save
dialog, plus a markdown clipboard copy — from the Listen document only,
per spec `docs/superpowers/specs/2026-10-08-transcript-export-design.md`.

**Architecture:** The webview builds the markdown (`transcriptMarkdown`
in `listen/model.ts` — same block model as `transcriptCopyText`); one
generic Rust command `save_text_file` owns the OS piece (native `rfd`
save dialog + `fs::write`), `Gate::Main`-guarded. The trigger is an
export dropdown on the `SpeakerFilter` row (`ChatMsgMenu` pattern).

**Tech Stack:** Rust/Tauri 2 + `rfd`; React/TS webview (`bun`), lucide
icons via the `@marvis/ui` barrel.

## Global Constraints

- `bun` for all JS/package commands; `cargo` in `apps/native/src-tauri`.
- IPC: typed wrappers in `src/lib/commands.ts` — never raw `invoke` in
  components. Rust params snake_case / JS args camelCase.
- Commands return `Result<T, String>`; mutating commands check
  `*state.gate.lock() != Gate::Main` (warn + no-op, matching the
  palette commands); `parking_lot` guards never held across `.await`.
- Blocking OS calls (the dialog) run on
  `tauri::async_runtime::spawn_blocking` — never the async executor.
- Webview imports: `@/` alias cross-directory, `./` same-dir; arrow
  functions + named exports; icons only `Icon`-suffixed lucide names
  through the `@marvis/ui` barrel (`packages/ui/src/index.ts`).
- Verification: `cargo test` + `cargo clippy` in `src-tauri`;
  `bun test` + `bun run check-types` + `bun run build` in `apps/native`.

---

### Task 1: `save_text_file` — Rust save-dialog command

**Files:**

- Modify: `apps/native/src-tauri/Cargo.toml` (add `rfd` dep)
- Modify: `apps/native/src-tauri/src/lib.rs` (command + sanitizer +
  registration + tests)

**Interfaces:**

- Produces: `save_text_file(suggested_name: String, contents: String)
  -> Result<Option<String>, String>` — `Ok(None)` = user cancel OR
  gate != Main; `Ok(Some(path))` = written. Consumed by Task 3's
  `saveTextFile` wrapper (JS arg `suggestedName` — camelCase bridge).
- `sanitize_suggested_name(&str) -> String` — private, unit-tested in
  the lib.rs test module.

- [ ] **Step 1: Add the `rfd` dependency**

In `apps/native/src-tauri/Cargo.toml`, add to `[dependencies]` (next to
the other `tauri-plugin-*` entries, alphabetical):

```toml
rfd = "0.15"
```

Run: `cd apps/native/src-tauri && cargo check`
Expected: PASS (resolves + compiles).

- [ ] **Step 2: Write the failing tests**

In the `#[cfg(test)] mod tests` block inside `lib.rs` (search for
`preset_commands_and_dispatch_are_in_the_contract` and add after it):

```rust
/// The export surface: `save_text_file` is registered in
/// `generate_handler!`, gate-guarded like the palette commands, and
/// runs its blocking dialog off the async executor. `concat!` keeps
/// the literal out of this file's text so the assert can't
/// self-satisfy.
#[test]
fn export_command_is_registered_and_gate_guarded() {
    let source = include_str!("lib.rs");
    assert!(source.contains(concat!("save", "_text_file,")));
    let body = source
        .split("async fn save_text_file(")
        .nth(1)
        .and_then(|rest| rest.split("\n/// ").next())
        .expect("save_text_file body not found");
    assert!(
        body.contains("*state.gate.lock() != Gate::Main"),
        "save_text_file must check Gate::Main"
    );
    assert!(
        body.contains("spawn_blocking"),
        "save_text_file's dialog must run off the async executor"
    );
}

#[test]
fn suggested_name_sanitizes_separators_controls_and_caps() {
    assert_eq!(sanitize_suggested_name("a/b\\c.md"), "abc.md");
    assert_eq!(sanitize_suggested_name("n\u{0}ame.md"), "name.md");
    assert_eq!(sanitize_suggested_name("   "), "marvis-export.md");
    assert_eq!(sanitize_suggested_name(""), "marvis-export.md");
    assert_eq!(sanitize_suggested_name(&"x".repeat(200)).len(), 80);
    assert_eq!(sanitize_suggested_name("notes.md"), "notes.md");
}
```

Run: `cd apps/native/src-tauri && cargo test suggested_name`
Expected: FAIL — `sanitize_suggested_name` not defined.

- [ ] **Step 3: Implement the command + sanitizer**

In `lib.rs`, insert immediately before the
`// Commands — config / app` divider (after `session_resume`, ~line 2537):

```rust
/// Document egress (export): a native save dialog + write. Deliberately
/// dumb — the webview builds the document; this owns the two things a
/// webview can't do without an fs capability. `None` = user cancel
/// (also the gate-drop result: a crafted invoke gets the same nothing).
#[tauri::command]
async fn save_text_file(
    app: AppHandle,
    suggested_name: String,
    contents: String,
) -> Result<Option<String>, String> {
    let state = app.state::<AppState>();
    if *state.gate.lock() != Gate::Main {
        log::warn!("save_text_file dropped while gate != Main");
        return Ok(None);
    }
    tauri::async_runtime::spawn_blocking(move || {
        let Some(path) = rfd::FileDialog::new()
            .set_file_name(sanitize_suggested_name(&suggested_name))
            .add_filter("Markdown", &["md"])
            .save_file()
        else {
            return Ok(None);
        };
        std::fs::write(&path, contents).map_err(|e| e.to_string())?;
        Ok(Some(path.to_string_lossy().into_owned()))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// The dialog's suggested name: path separators and control chars can't
/// smuggle a directory choice past the picker; the cap keeps the dialog
/// field sane. Never a path — just a filename.
fn sanitize_suggested_name(name: &str) -> String {
    let clean: String = name
        .chars()
        .filter(|c| !matches!(c, '/' | '\\') && !c.is_control())
        .take(80)
        .collect();
    let clean = clean.trim();
    if clean.is_empty() {
        "marvis-export.md".to_string()
    } else {
        clean.to_string()
    }
}
```

Then register in `tauri::generate_handler!` — add `save_text_file,`
after `session_resume,` (with the other session commands).

Run: `cd apps/native/src-tauri && cargo test`
Expected: PASS — both new tests plus the existing suite.

- [ ] **Step 4: Clippy + commit**

Run: `cd apps/native/src-tauri && cargo clippy -- -D warnings`
Expected: PASS, no warnings.

```bash
git add apps/native/src-tauri/Cargo.toml apps/native/src-tauri/Cargo.lock apps/native/src-tauri/src/lib.rs
git commit -m "Export: save_text_file — native save dialog + write, Gate::Main-guarded"
```

---

### Task 2: `transcriptMarkdown` + `exportFileName` — model.ts builders

**Files:**

- Modify: `apps/native/src/components/listen/model.ts` (append builders)
- Modify: `apps/native/src/components/listen/model.test.ts` (append tests)

**Interfaces:**

- Consumes: existing `TurnBlock`, `elapsedLabel`, `timeLabel`,
  `blockText`, `format` (date-fns, already imported), and
  `ListenSummaryPayload` (`{tldr, bullets, follow_ups, topic}` —
  already imported via `@/lib/events`).
- Produces: `transcriptMarkdown(blocks: TurnBlock[], meta: { title?:
  string | null; startedAt: number | null; stt?: string | null },
  summary: ListenSummaryPayload | null): string` and
  `exportFileName(topic: string | null | undefined, startedAt: number |
  null): string`. Consumed by Task 3.

- [ ] **Step 1: Write the failing tests**

Append to `apps/native/src/components/listen/model.test.ts` — change
the model import to
`import { buildBlocks, exportFileName, timeLabel, transcriptMarkdown, type Turn } from './model';`,
add `import { format } from 'date-fns';` and
`import type { ListenSummaryPayload } from '@/lib/events';`, reuse the
file's existing `turn()` fixture, then:

```ts
describe('transcriptMarkdown', () => {
  const summary: ListenSummaryPayload = {
    tldr: 'Talked about the roadmap.',
    bullets: ['Phase 1 continues', 'Export ships'],
    follow_ups: ['Write the plan'],
    topic: 'Weekly standup',
  };
  const meta = {
    title: 'Weekly standup',
    startedAt: 1_700_000_000,
    stt: 'whisper tiny',
  };

  test('title, meta line, summary, and transcript sections', () => {
    const md = transcriptMarkdown(
      buildBlocks([
        turn(1_700_000_042, {
          text: 'first turn',
          speaker: 'me',
          speaker_idx: null,
        }),
        turn(1_700_000_134, { text: 'reply' }),
      ]),
      meta,
      summary,
    );
    expect(md).toBe(
      [
        '# Weekly standup',
        '',
        `_${format(1_700_000_000 * 1000, 'MMM d, yyyy · HH:mm')} · whisper tiny_`,
        '',
        '## Summary',
        '',
        'Talked about the roadmap.',
        '',
        '- Phase 1 continues',
        '- Export ships',
        '',
        '### Follow-ups',
        '',
        '- Write the plan',
        '',
        '## Transcript',
        '',
        '- **[0:42] You:** first turn',
        '- **[2:14] Speaker 1:** reply',
        '',
      ].join('\n'),
    );
  });

  test('degrades: no summary, no stt, wall-clock stamps without startedAt', () => {
    const md = transcriptMarkdown(
      buildBlocks([
        turn(1_700_000_042, {
          text: 'hi',
          speaker: 'me',
          speaker_idx: null,
        }),
      ]),
      { startedAt: null },
      null,
    );
    expect(md).not.toContain('## Summary');
    expect(md).not.toContain('Follow-ups');
    expect(md).toContain('# Listen session');
    expect(md).toContain(`- **[${timeLabel(1_700_000_042)}] You:** hi`);
  });

  test('no turns drops the Transcript section; empty follow_ups drops its heading', () => {
    const md = transcriptMarkdown([], meta, { ...summary, follow_ups: [] });
    expect(md).not.toContain('## Transcript');
    expect(md).not.toContain('Follow-ups');
    expect(md).toContain('## Summary');
  });

  test('a riding interim is excluded — export is finals only', () => {
    const md = transcriptMarkdown(
      buildBlocks([
        turn(1_700_000_042, { text: 'done' }),
        turn(1_700_000_050, {
          text: 'draft',
          interim: true,
          final: false,
        }),
      ]),
      meta,
      null,
    );
    expect(md).toContain('done');
    expect(md).not.toContain('draft');
  });
});

describe('exportFileName', () => {
  test('slugifies the topic, stamps from startedAt', () => {
    const name = exportFileName('Weekly Standup!', 1_700_000_000);
    expect(name).toBe(
      `marvis-weekly-standup-${format(1_700_000_000 * 1000, 'yyyyMMdd-HHmm')}.md`,
    );
  });

  test('fallback slug + long topics cap at 40 chars', () => {
    expect(exportFileName(null, null)).toMatch(
      /^marvis-listen-\d{8}-\d{4}\.md$/,
    );
    const long = exportFileName('a'.repeat(60), null);
    expect(long).toMatch(/^marvis-a{40}-\d{8}-\d{4}\.md$/);
  });
});
```

Run: `cd apps/native && bun test model.test.ts`
Expected: FAIL — `transcriptMarkdown`/`exportFileName` not defined.

- [ ] **Step 2: Implement the builders**

Append to `apps/native/src/components/listen/model.ts`:

```ts
/** Markdown document — the export/copy counterpart of
 *  `transcriptCopyText`: same block model and the same stamp rule
 *  (elapsed off `startedAt`, wall-clock without), richer shape.
 *  Sections degrade — a missing summary or an empty transcript drops
 *  its heading rather than rendering an empty shell. */
export const transcriptMarkdown = (
  blocks: TurnBlock[],
  meta: { title?: string | null; startedAt: number | null; stt?: string | null },
  summary: ListenSummaryPayload | null,
): string => {
  const metaLine = [
    meta.startedAt != null
      ? format(meta.startedAt * 1000, 'MMM d, yyyy · HH:mm')
      : null,
    meta.stt,
  ]
    .filter(Boolean)
    .join(' · ');
  const out: string[] = [`# ${meta.title?.trim() || 'Listen session'}`];
  if (metaLine) out.push('', `_${metaLine}_`);
  if (summary) {
    out.push('', '## Summary', '', summary.tldr, '');
    out.push(...summary.bullets.map((b) => `- ${b}`));
    if (summary.follow_ups.length) {
      out.push('', '### Follow-ups', '');
      out.push(...summary.follow_ups.map((f) => `- ${f}`));
    }
  }
  if (blocks.length) {
    out.push('', '## Transcript', '');
    for (const b of blocks) {
      const stamp =
        meta.startedAt != null
          ? elapsedLabel(b.ts - meta.startedAt)
          : timeLabel(b.ts);
      // Finals only — a riding interim isn't in the exported document.
      out.push(
        `- **[${stamp}] ${b.name}:** ${b.finals.map((t) => t.text).join(' ')}`,
      );
    }
  }
  out.push('');
  return out.join('\n');
};

/** `marvis-{slug}-YYYYMMDD-HHmm.md` — slug off the doc title (`listen`
 *  fallback); the stamp comes from `startedAt`, else now. */
export const exportFileName = (
  topic: string | null | undefined,
  startedAt: number | null,
): string => {
  const slug = (topic ?? '')
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-+|-+$/g, '')
    .slice(0, 40)
    .replace(/-+$/g, '');
  const stamp = format(
    (startedAt ?? Date.now() / 1000) * 1000,
    'yyyyMMdd-HHmm',
  );
  return `marvis-${slug || 'listen'}-${stamp}.md`;
};
```

Run: `cd apps/native && bun test model.test.ts`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add apps/native/src/components/listen/model.ts apps/native/src/components/listen/model.test.ts
git commit -m "Export: transcriptMarkdown + exportFileName builders in the listen doc model"
```

---

### Task 3: Export menu — ui icon, command wrapper, SpeakerFilter, ListenSection

**Files:**

- Modify: `packages/ui/src/index.ts` (export `ShareIcon`)
- Modify: `apps/native/src/lib/commands.ts` (add `saveTextFile` in the
  sessions section, after `sessionResume`)
- Modify: `apps/native/src/components/listen/SpeakerFilter.tsx`
- Modify: `apps/native/src/components/ListenSection.tsx`

**Interfaces:**

- Consumes: Task 1's `save_text_file` command; Task 2's
  `transcriptMarkdown`/`exportFileName`; existing `raise` helper
  (`commands.ts` — fire-and-forget `alert_show`); the `ChatMsgMenu`
  dropdown pattern (`components/ChatMsgMenu.tsx`).
- Produces: `saveTextFile(suggestedName: string, contents: string):
  Promise<string | null>`; `SpeakerFilter` props `exported: boolean`,
  `onCopyMarkdown: () => void`, `onSaveMarkdown: () => void`.

- [ ] **Step 1: Export `ShareIcon` from the ui barrel**

In `packages/ui/src/index.ts`, add to the lucide export list
(alphabetically between `SettingsIcon` and `ShieldIcon`):

```ts
  ShareIcon,
```

- [ ] **Step 2: Add the `saveTextFile` wrapper**

In `apps/native/src/lib/commands.ts`, in the `// sessions` section
after `sessionResume`:

```ts
/** Document export: native save dialog + write, `null` on cancel. The
 *  name is only a suggestion — the picker owns the final path. */
export const saveTextFile = (suggestedName: string, contents: string) =>
  invoke<string | null>('save_text_file', { suggestedName, contents });
```

- [ ] **Step 3: `SpeakerFilter` export dropdown**

In `apps/native/src/components/listen/SpeakerFilter.tsx`:

- Add `import { useState } from 'react';` at the top and add
  `ShareIcon` to the `@marvis/ui` import.
- Extend the props:

```ts
  exported,
  onCopyMarkdown,
  onSaveMarkdown,
}: {
  speakers: { key: string; name: string; color: string }[];
  active: string | null;
  count: number;
  /** Formatted `m:ss` duration — null while no session is on screen. */
  elapsed: string | null;
  copied: boolean;
  exported: boolean;
  onPick: (key: string | null) => void;
  onCopy: () => void;
  onCopyMarkdown: () => void;
  onSaveMarkdown: () => void;
}) => {
```

- The component is currently a pure arrow returning JSX — convert the
  signature to a block body (`=> { const [exportOpen, setExportOpen] =
  useState(false); return ( ... ); };`) and add, immediately after the
  existing copy `<button>` (before the closing `</div>`):

```tsx
    <div className='relative flex-none'>
      <button
        type='button'
        className={cn(ICON_BTN, 'size-5')}
        title='Export transcript'
        aria-label='Export transcript'
        aria-expanded={exportOpen}
        onClick={() => setExportOpen((o) => !o)}>
        {exported ? (
          <CheckIcon className='size-3 text-accent' />
        ) : (
          <ShareIcon className='size-3' />
        )}
      </button>
      {exportOpen && (
        <>
          <button
            type='button'
            aria-hidden
            tabIndex={-1}
            onClick={() => setExportOpen(false)}
            className='fixed inset-0 z-10 cursor-default border-0 bg-transparent'
          />
          <div className='absolute right-0 top-full z-20 mt-1 min-w-36 rounded-xl border border-border bg-[color-mix(in_oklch,var(--surface)_88%,transparent)] py-1 text-[12px] text-foreground shadow-md backdrop-blur-lg'>
            <button
              type='button'
              className='flex w-full cursor-pointer items-center border-0 bg-transparent px-2.5 py-1.5 text-left text-foreground enabled:hover:bg-fg-soft'
              onClick={() => {
                setExportOpen(false);
                onCopyMarkdown();
              }}>
              Copy markdown
            </button>
            <button
              type='button'
              className='flex w-full cursor-pointer items-center border-0 bg-transparent px-2.5 py-1.5 text-left text-foreground enabled:hover:bg-fg-soft'
              onClick={() => {
                setExportOpen(false);
                onSaveMarkdown();
              }}>
              Save .md…
            </button>
          </div>
        </>
      )}
    </div>
```

Update the doc comment's meta description: "the elapsed recording
time, and transcript copy + export".

- [ ] **Step 4: `ListenSection` wiring**

In `apps/native/src/components/ListenSection.tsx`:

- Extend the commands import with `saveTextFile`, add `raise` to the
  same import, and extend the model import with `transcriptMarkdown`
  and `exportFileName`.
- Add state next to `copiedAll`: `const [exported, setExported] =
  useState(false);`
- Add the handlers next to `copyAll` (uses `blocks` — the unfiltered
  doc, same as copy; `live ? engine : viewing?.stt` mirrors the
  subtitle's engine rule):

```ts
  /** The export document — markdown off the same blocks the copy path
   *  uses; the speaker filter never narrows export scope. */
  const markdownDoc = () =>
    transcriptMarkdown(
      blocks,
      {
        title: summary?.topic,
        startedAt,
        stt: live ? engine : (viewing?.stt ?? null),
      },
      summary,
    );

  const copyMarkdown = () => {
    void navigator.clipboard
      .writeText(markdownDoc())
      .then(() => {
        setExported(true);
        window.setTimeout(() => setExported(false), 1500);
      })
      .catch(() => {});
  };

  const saveMarkdown = () => {
    void saveTextFile(exportFileName(summary?.topic, startedAt), markdownDoc())
      .then((path) => {
        if (path !== null) {
          setExported(true);
          window.setTimeout(() => setExported(false), 1500);
        }
      })
      .catch((e) => raise(typeof e === 'string' ? e : 'Export failed'));
  };
```

- Pass the new props on the `<SpeakerFilter>` element:

```tsx
        exported={exported}
        onCopyMarkdown={copyMarkdown}
        onSaveMarkdown={saveMarkdown}
```

- [ ] **Step 5: Typecheck + build**

Run: `cd apps/native && bun run check-types && bun run build`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add packages/ui/src/index.ts apps/native/src/lib/commands.ts apps/native/src/components/listen/SpeakerFilter.tsx apps/native/src/components/ListenSection.tsx
git commit -m "Export: markdown copy + .md save menu on the listen doc"
```

---

### Task 4: End-to-end verification

**Files:** none — manual + suite verification.

- [ ] **Step 1: Full test suites**

Run: `cd apps/native/src-tauri && cargo test && cargo clippy -- -D warnings`
Run: `cd apps/native && bun test && bun run check-types`
Expected: all PASS.

- [ ] **Step 2: Manual QA via `bun run build:dev`**

Checklist (from the spec):

- Live session: export mid-capture writes a `.md` with the finals
  captured so far; `Copy markdown` pastes identical content.
- Viewed doc (from History): `.md` renders title / meta line /
  Summary / Follow-ups / Transcript in an md-aware editor.
- Cancel in the save dialog writes nothing and shows no feedback.
- A no-summary, no-turns session still writes `# Listen session`.
- Plain-text copy button behavior unchanged.
