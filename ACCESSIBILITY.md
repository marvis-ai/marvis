# Accessibility

Accessibility is a core priority for **Marvis**. We want everyone — including
people with disabilities and people using assistive technology — to be able to
use the desktop app, read the website, and contribute to this project.

This document explains our accessibility commitment, how contributors can help
us uphold it, and how to report accessibility issues.

## Priorities

- **Accessibility target:** We work toward
  [WCAG 2.2 Level AA](https://www.w3.org/TR/WCAG22/) across the desktop app's
  webview UI (`apps/native`) and the marketing site (`apps/web`). This target
  guides our work but is not a claim of verified conformance.
- **Keyboard support** — every interactive element must be reachable and
  operable with a keyboard alone, with a visible focus indicator and a logical
  tab order. The desktop app is keyboard-driven: a rebindable global hotkey
  (`Cmd`/`Ctrl+Alt+Space`) shows and hides the bar's input, and every in-bar
  action has a key (see [Hotkeys](README.md#hotkeys)). Pointer drag must never
  be the only way to operate a control — the bar also docks via Settings → Bar
  edge picker.
- **Screen reader requirements** — the webview must use semantic HTML and a
  logical heading hierarchy. Navigation testing targets are VoiceOver (macOS),
  NVDA, JAWS, and Narrator (Windows), and Orca (Linux); support has not been
  verified.
- **Motion is state — and optional** — per `apps/native/DESIGN.md`, animation
  exists only to communicate state changes. `prefers-reduced-motion` stills the
  breathing idle, capsule morphs, waveforms, and the caret; new motion must
  respect it too.
- **Readable content** — plain language, descriptive link text, semantic
  lists, meaningful alternative text, and sufficient contrast against the
  bar's translucent material.
- **Live transcripts** — Listen produces real-time, speaker-labeled
  transcripts and rolling summaries that double as meeting captions. Keep them
  legible: sufficient contrast, no color-only speaker labels, and text that
  survives window resizing.

## Contributor expectations

If you are contributing content or code, please follow these guardrails so we
don't regress accessibility:

- **Testing**
  - For UI changes, test with an automated accessibility tool (such as
    [axe DevTools](https://www.deque.com/axe/devtools/) or the
    [GitHub Accessibility Scanner](https://github.com/github/accessibility-scanner)).
  - Verify in the real surface: run the desktop app with `bun run build:dev`,
    not only the Vite/Next dev servers — webview focus behavior, hotkeys, and
    native materials differ from a plain browser tab.
  - Do at least one keyboard-only pass on any changes involving interactive UI
    elements.
    - Tab order is logical (no jumps, no traps, reaches all interactive
      controls).
    - Visible focus indicator is always present and has sufficient contrast —
      including on the bar's translucent background.
    - All actions work by keyboard (Tab/Shift+Tab, Enter, Space, arrow keys
      where expected).
    - No keyboard trap (can move into and out of modals, menus, popovers,
      editors, and the chat panel).
  - Spot-check screen reader behavior for new components or significant
    content changes, with the screen reader native to your OS.
    - Controls have clear, accessible labels (programmatic name that matches
      the action/field purpose).
    - Custom controls expose proper semantics/state (role, name, value;
      toggles/expanded/selected announced).
    - Dynamic updates are announced appropriately — streaming Ask responses,
      live transcript lines, errors, and toasts via ARIA live regions as
      needed.
  - Keep motion honest: new animation must communicate state and must be
    disabled under `prefers-reduced-motion: reduce`.
- **Documentation and content**
  - Use a logical heading hierarchy (do not skip levels).
  - Use unique, descriptive link text (avoid "click here" / "read more").
  - Provide meaningful alternative text for images; refer to the
    [W3C alt Decision Tree](https://www.w3.org/WAI/tutorials/images/decision-tree/).
  - For complex images or diagrams, include a text alternative nearby.
  - For videos, provide captions and a transcript.
  - Don't use color as the only way to convey meaning — including speaker
    labels and provider/status indicators.
  - Content reflows without loss of information or functionality (test at
    200% and with narrow widths; the bar must stay usable at its smallest
    size).
- **CI/CD**
  - PRs may be blocked if they introduce accessibility violations detected by
    our linting or scanning workflows.
  - Resolve flagged issues, or document why a violation cannot be addressed in
    the PR description.

## Reporting accessibility issues

If you run into an accessibility barrier, please let us know — we treat
accessibility reports as expertise, not complaints.

1. **Open an issue** in the
   [issue tracker](https://github.com/MarvisLLC/marvis/issues) (the bug report
   template works — just mention accessibility in the title).
2. Include, when possible:
   - What you were trying to do and what went wrong.
   - Which surface: the floating bar, chat/listen panel, onboarding,
     settings window, or the website (with the page URL).
   - Steps to reproduce.
   - Your operating system, app version or browser, and assistive technology
     (with versions).
   - A screen recording or screenshot, if you're comfortable sharing one.
   - Severity, using the [defined taxonomy](#severity).

### Severity

- **Critical:** Prevents you from completing a core task (for example, you
  cannot send an Ask or read a transcript at all).
- **Serious:** Significant difficulty, but a workaround exists.
- **Moderate:** Annoyance or inconsistent experience.
- **Minor:** Minor issue with minimal impact on usability.

### How we respond

- We will acknowledge the reporter's experience promptly, respectfully, and
  constructively — treating accessibility reports as valuable project
  expertise rather than complaints.
- We will not require reporters to disclose a diagnosis or other personal
  information.
- We will explain next steps, known limitations, and relevant dependencies.
- Where possible, we will provide a workaround while a fix is in progress.
- We will provide updates when the status or expected timeline changes (for
  example, "We're working on this — tracking in #123").
- We may ask the reporter to confirm that a fix resolves the barrier before
  closing the issue.
- We will thank the reporter for helping improve the project.

### Resolution expectations

Resolution expectations help reporters understand when action is likely and
help maintainers prioritize issues consistently. After triage, we assign each
issue a severity, owner, and target resolution date.

- **Critical:** Resolve within 30 days.
- **Serious:** Resolve within 60 days.
- **Moderate:** Resolve within 90 days.
- **Minor:** Resolve within 90 days.

These targets begin when the issue is opened. If we cannot meet a target, we
will explain the delay, share any available workaround, and provide a revised
target date.

## Ownership and maintenance

Accessibility is owned by **the Marvis maintainers** ([@MarvisLLC](https://github.com/MarvisLLC)).

### Responsibilities

The accessibility owner is responsible for:

- Triaging accessibility reports.
- Tracking accessibility work and known barriers.
- Sharing status updates and escalating unresolved accessibility risks to
  project maintainers.
- Keeping the project's accessibility documentation current.

## Supported environments

The desktop app renders its UI in each platform's system webview; assistive
technology support rides on that engine:

| Platform | Webview engine | Assistive technology |
| --- | --- | --- |
| macOS | WKWebView | VoiceOver |
| Windows | WebView2 | NVDA, JAWS, Narrator |
| Linux | WebKitGTK | Orca |

The marketing site is intended to work across:

- **Web (desktop):** latest two versions of Chrome, Edge, Firefox, and Safari.
- **Web (mobile):** latest two versions of Mobile Safari and Chrome on
  Android.
- **Assistive technology:** VoiceOver (macOS / iOS), NVDA (Windows), JAWS
  (Windows), TalkBack (Android).

Partial-support notes:

- Responses streamed from third-party LLM providers are generated content —
  heading structure, list markup, and description quality vary by model and
  are supported on a best-effort basis.
- OS-level consent dialogs (the macOS Screen Recording prompt, the Linux XDG
  portal picker) are native system UI outside our direct control.

## Known limitations

- The bar repositions by pointer drag or Settings → Bar edge picker; there is
  no keyboard-only move/scroll/click-through shortcut yet.
- The bar is translucent and floats over arbitrary screen content — contrast
  is tuned against its native material, but unusual backdrops may reduce
  legibility. Please report combinations that fail.
- Screen-aware Ask depends on a vision-capable model to describe frames;
  description quality is a model property, not something we can guarantee.
- Some older documents and design specs under `docs/` may not yet meet every
  guideline (for example, missing alternative text or non-descriptive link
  text). We are working through these as we update content.

**Note**: Please open an accessibility issue if you find one.

## Feedback

Accessibility is an ongoing practice, not a one-time fix. If you have
suggestions for improving this statement or our practices, please open an
issue or a pull request. Thank you for helping make Marvis usable by
everyone.
