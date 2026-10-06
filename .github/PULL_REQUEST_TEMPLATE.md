<!-- markdownlint-disable MD041 -->

## Summary

<!-- What does this PR change and why? Link issues, e.g. "Fixes #123". -->

## Type of change

- [ ] Bug fix
- [ ] New feature
- [ ] Refactor / cleanup
- [ ] Documentation
- [ ] CI / build / tooling

## Test plan

<!-- How was this verified? Check what applies. -->

- [ ] `bun run lint`
- [ ] `bun run check-types`
- [ ] `cargo test` (in `apps/native/src-tauri`)
- [ ] Manually verified on: <!-- e.g. macOS 15, Windows 11, Ubuntu 24.04 -->

## Checklist

- [ ] Follows `AGENTS.md` conventions (bun, arrow-function components with
      named exports, `Icon`-suffixed icons via `@marvis/ui`)
- [ ] Command/event contract updated on both sides
      (`src/lib/commands.ts` / `events.ts` + Rust handlers) if touched
- [ ] No secrets, keys, or `~/.marvis` user data committed
- [ ] Screenshots or a recording attached for UI changes
- [ ] Docs updated (README / AGENTS.md / release notes) if behavior changed
