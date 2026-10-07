# Contributing to Marvis

Thanks for your interest in contributing to Marvis — a privacy-first AI
co-pilot for the desktop, built with Tauri 2, Rust, and React. This guide
covers the workflow; the coding and architecture rules live in
[AGENTS.md](AGENTS.md), which is the source of truth for conventions in this
codebase.

## Ways to contribute

- **Bug reports** — open an [issue][issues] using the bug report template.
- **Feature requests** — open an [issue][issues] using the feature request
  template, or start a [discussion][discussions] for early-stage ideas.
- **Questions** — ask in [GitHub Discussions][discussions], not issues.
- **Security reports** — never in public. See [SECURITY.md](SECURITY.md).
- **Pull requests** — welcome for bugs and agreed-upon features. For
  anything large, open an issue or discussion first so we can align on
  direction before you invest the time.

## Development setup

Requirements: [Bun](https://bun.sh) 1.3+, the stable
[Rust](https://rustup.rs) toolchain, and your platform's build tools — see
the [README](README.md#requirements) for the per-OS list.

```bash
bun install          # install workspace dependencies
bun run build:dev    # run the desktop app (stages the whisper sidecar)
```

Other useful root scripts:

```bash
bun run lint         # lint all workspaces
bun run check-types  # typecheck all workspaces
bun run test         # workspace tests
```

Rust unit tests for the Tauri core:

```bash
cd apps/native/src-tauri && cargo test
```

## Before you open a PR

- Keep changes focused — one concern per PR.
- Match the conventions in [AGENTS.md](AGENTS.md): `bun` for all package
  management, arrow-function components with named exports, `Icon`-suffixed
  lucide imports through `@marvis/ui`, and the typed IPC boundary
  (`apps/native/src/lib/commands.ts` + `events.ts` — never a raw `invoke()`
  or `listen()` in components).
- The command/event contract is dual-sided: when you change a command or
  event, update the TypeScript wrappers and the Rust handlers together, and
  update the contract tests in `src-tauri/src/lib.rs`.
- Run `bun run lint` and `bun run check-types`. For Rust changes, run
  `cargo test` in `apps/native/src-tauri`.
- Commit style: `scope: short summary` — see `git log` for examples
  (`native:`, `web:`, `ui:`, `ci:`, `docs:`).
- Fill in the pull request template, link the issue it closes, and attach
  screenshots or a recording for UI changes.

## License

By contributing, you agree that your contributions are licensed under the
[Apache License 2.0](LICENSE).

[issues]: https://github.com/marvis-ai/marvis/issues
[discussions]: https://github.com/marvis-ai/marvis/discussions
