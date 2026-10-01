# Bundled Whisper CLI Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use
> superpowers:subagent-driven-development (recommended) or
> superpowers:executing-plans to implement this plan task-by-task. Steps use
> checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build and bundle architecture-specific `whisper-cli` executables with
Marvis so downloaded Whisper models work without users installing the CLI
separately.

**Architecture:** A pinned official whisper.cpp source revision (`v1.9.2`) is
built in macOS CI for arm64 and x86_64. Tauri packages the resulting binaries as
architecture-specific external binaries/resources. Rust resolves the bundled
executable first, then preserves PATH/Homebrew/user-local fallbacks; Listen and
Settings receive only safe source/availability status, never binary paths in
public events.

**Tech Stack:** GitHub Actions macOS runners, CMake, C/C++/Metal, whisper.cpp
v1.9.2, Tauri 2 external binaries/resources, Rust `std::process::Command`,
existing Tokio/reqwest/Tauri runtime, React 19/Vite.

## Global Constraints

- Bundle `whisper-cli` with the Marvis app; do not download the CLI at
  model-download time.
- Build official whisper.cpp source in macOS CI; do not use community binaries
  or runtime compilation.
- Build and bundle arm64 and x86_64 macOS binaries; resolve the one matching the
  running app architecture.
- Use pinned whisper.cpp revision `v1.9.2`; do not build an unversioned `main`
  checkout.
- Verify CI-produced binaries with SHA-256 before packaging and code-sign them
  as part of the Marvis app signing flow.
- Runtime resolution order is bundled binary, PATH,
  `/opt/homebrew/bin/whisper-cli`, then
  `~/.marvis/models/whisper/bin/whisper-cli`.
- The bundled binary is never copied into `~/.marvis/models/whisper/bin/`.
- The app must not silently build whisper.cpp at runtime; source builds happen
  only in the pinned macOS CI packaging workflow.
- Downloaded Whisper model files remain data files and are never executed.
- API keys remain in `keys.json`; no keys, PCM, transcripts, URLs, or local
  paths enter public Listen events.
- Preserve existing Listen setup errors, explicit Stop behavior, and
  card-collapse/reopen behavior.
- Do not modify unrelated prompt redesign, liquid-glass, or dependency-warning
  work.

---

## File Map

### CI/build files to create or modify

- Create: `.github/workflows/whisper-cli.yml` — build arm64/x86_64 CLI artifacts
  from whisper.cpp v1.9.2, verify SHA-256, and publish artifacts for packaging.
- Create: `apps/native/src-tauri/scripts/build-marvis.sh` — deterministic
  local/CI source build and artifact naming.
- Create: `apps/native/src-tauri/binaries/.gitkeep` only if the repository needs
  the directory present before CI artifacts are copied; never commit binary
  files.
- Modify: `apps/native/package.json` only if a release/build script needs to
  invoke the CI artifact preparation locally.

### Tauri/Rust files to modify

- `apps/native/src-tauri/tauri.conf.json` — declare the architecture-specific
  Whisper external binary/resource packaging.
- `apps/native/src-tauri/src/stt/whisper.rs` — source-aware bundled-first binary
  resolution and safe status.
- `apps/native/src-tauri/src/voice_models.rs` — pass/consume bundled binary
  status without exposing paths in download events.
- `apps/native/src-tauri/src/listen.rs` — use bundled-aware Whisper
  preflight/provider construction.
- `apps/native/src-tauri/src/lib.rs` — pass Tauri resource context to
  Listen/model status and expose source-safe status.
- `apps/native/src-tauri/src/paths.rs` — add only the resource/bundled-binary
  path abstraction needed by the resolver.

### Frontend files to modify

- `apps/native/src/lib/commands.ts` — extend Whisper status type with safe
  source/availability fields if needed.
- `apps/native/src/components/prefs/VoiceSetup.tsx` — render
  bundled/custom/missing source labels without showing sensitive paths in
  download/Listen events.
- `apps/native/src/components/ListenSection.tsx` — preserve setup guidance while
  using the authoritative bundled-aware status.

---

## Task 1: Build pinned whisper.cpp CLI artifacts in macOS CI

**Files:**

- Create: `.github/workflows/whisper-cli.yml`
- Create: `apps/native/src-tauri/scripts/build-marvis.sh`
- Test: CI shell validation and local script dry-run/metadata checks

**Interfaces:**

- Produces `whisper-cli-aarch64-apple-darwin` and
  `whisper-cli-x86_64-apple-darwin` artifacts.
- Uses source revision `v1.9.2` from the official whisper.cpp repository.
- Emits SHA-256 checksum files alongside each artifact.
- Does not commit generated binaries to Git.

- [ ] **Step 1: Write failing artifact metadata checks.** Add shell assertions
      in the build script/test path that fail when the source revision is not
      exactly `v1.9.2`, output names are not the two expected target names, or
      either output is missing/non-executable.

- [ ] **Step 2: Run the metadata check before implementation.**

```bash
cd apps/native/src-tauri
bash scripts/build-marvis.sh --check-only
```

Expected: FAIL because the script does not exist.

- [ ] **Step 3: Implement deterministic source build.** The script must:

```text
SOURCE_REPO=https://github.com/ggml-org/whisper.cpp.git
SOURCE_REV=v1.9.2
TARGET=aarch64-apple-darwin | x86_64-apple-darwin
```

For each target, clone/fetch only the pinned revision into a temporary build
directory, configure CMake with examples enabled, tests disabled, and Metal
enabled on macOS, then build the `whisper-cli` target in Release mode. Copy only
the resulting executable to the exact target-triple artifact name and run
`file`, `test -x`, and `shasum -a 256`.

- [ ] **Step 4: Add the GitHub Actions workflow.** Use separate macOS
      jobs/runners appropriate to the target architectures, invoke the shared
      script, upload each binary/checksum artifact, and fail if the checksum or
      architecture check fails. The workflow must not publish unpinned source
      output or silently substitute a different target.

- [ ] **Step 5: Run static script validation locally.**

```bash
bash -n apps/native/src-tauri/scripts/build-marvis.sh
bash apps/native/src-tauri/scripts/build-marvis.sh --check-only
```

Expected: PASS for syntax and metadata validation; a full local source build is
optional when the required CMake/Xcode toolchain is unavailable.

- [ ] **Step 6: Commit.**

```bash
git add .github/workflows/whisper-cli.yml \
  apps/native/src-tauri/scripts/build-marvis.sh \
  apps/native/src-tauri/binaries/.gitkeep
 git commit -m "build(native): add pinned Whisper CLI CI artifacts"
```

---

## Task 2: Integrate architecture-specific binaries into Tauri packaging

**Files:**

- Modify: `apps/native/src-tauri/tauri.conf.json`
- Modify: `apps/native/src-tauri/scripts/build-marvis.sh` if artifact staging
  needs one shared function
- Modify: `apps/native/package.json` only if a local staging command is required
- Test: Tauri config validation and staged package inspection

**Interfaces:**

- Tauri receives a staged `apps/native/src-tauri/binaries/whisper-cli` base path
  and selects the target-triple suffix required by Tauri external binaries.
- Release packaging fails if the current target’s binary is absent,
  non-executable, or has an unexpected checksum.

- [ ] **Step 1: Write a failing packaging-config test/check.** Assert the Tauri
      config declares the Whisper external binary/resource and that the staged
      target-specific file is required before packaging.

- [ ] **Step 2: Run the config check and verify failure.**

```bash
cd apps/native/src-tauri
cargo test --lib package_config -- --nocapture
```

Expected: FAIL until the packaging declaration/check exists.

- [ ] **Step 3: Configure Tauri external binary packaging.** Add the
      `whisper-cli` base external binary/resource entry in `tauri.conf.json`
      using Tauri’s target-triple convention, while retaining the existing macOS
      private API, transparent-window, and bundle settings. Do not add binary
      contents to Git.

- [ ] **Step 4: Implement staging validation.** Add a script mode that accepts
      the CI artifact for the current target, stages it under the expected Tauri
      binary name, verifies executable mode and SHA-256, and rejects
      missing/wrong-architecture files before `tauri build` runs.

- [ ] **Step 5: Inspect a staged bundle.** On an available macOS architecture,
      run the staging command and inspect the generated app/resource layout with
      `find`, `file`, and `test -x`. The check must prove the executable is
      inside the app bundle and has the expected architecture.

- [ ] **Step 6: Commit.**

```bash
git add apps/native/src-tauri/tauri.conf.json \
  apps/native/src-tauri/scripts/build-marvis.sh apps/native/package.json
 git commit -m "build(native): package bundled Whisper CLI"
```

---

## Task 3: Add bundled-first runtime resolution and source-aware status

**Files:**

- Modify: `apps/native/src-tauri/src/stt/whisper.rs`
- Modify: `apps/native/src-tauri/src/paths.rs`
- Modify: `apps/native/src-tauri/src/voice_models.rs`
- Test: `stt/whisper.rs`, `voice_models.rs`, and `paths.rs` tests

**Interfaces:**

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum WhisperBinarySource {
    Bundled,
    Path,
    Homebrew,
    User,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WhisperBinaryStatus {
    pub available: bool,
    pub source: Option<WhisperBinarySource>,
}

pub fn discover_with_bundled(
    bundled: Option<&Path>,
) -> Option<(PathBuf, WhisperBinarySource)>;
```

The existing `discover()` remains available for tests/fallback-only callers and
resolves the non-bundled paths. Production Listen/status paths use the bundled-
aware function with a Tauri-derived resource directory.

- [ ] **Step 1: Write failing resolver tests.** Cover bundled binary preferred
      over PATH/Homebrew/user-local, fallback order when bundled is absent,
      rejection of missing/non-regular/non-executable candidates, and source
      labels.

- [ ] **Step 2: Run focused tests and verify failure.**

```bash
cd apps/native/src-tauri
cargo test stt::whisper -- --nocapture
cargo test voice_models -- --nocapture
cargo test paths -- --nocapture
```

Expected: FAIL for bundled resolution/source status before implementation.

- [ ] **Step 3: Add resource-path helpers.** Add a small helper that accepts an
      externally resolved Tauri resource directory and returns the
      architecture-specific bundled executable candidate. Do not derive it from
      CWD. Keep Tauri-specific path acquisition outside the platform-neutral
      resolver so tests use ordinary temp paths.

- [ ] **Step 4: Implement bundled-first resolution.** Check the bundled
      candidate first, verify it is a regular executable file and matches the
      current target architecture where the platform permits inspection, then
      check PATH, Homebrew, and user-local locations. Discovery must not launch
      the process.

- [ ] **Step 5: Extend Whisper status safely.** Report `available` and `source`
      to local Settings status. Preserve the existing binary path only where the
      current local Settings DTO explicitly needs it; do not add paths/source to
      Listen events or provider requests.

- [ ] **Step 6: Run focused/full Rust checks.**

```bash
cargo test stt::whisper -- --nocapture
cargo test voice_models -- --nocapture
cargo test paths -- --nocapture
cargo test
cargo clippy --all-targets --all-features -- -D warnings
cargo check
```

- [ ] **Step 7: Commit.**

```bash
git add apps/native/src-tauri/src/stt/whisper.rs \
  apps/native/src-tauri/src/paths.rs apps/native/src-tauri/src/voice_models.rs
 git commit -m "feat(native): prefer bundled Whisper CLI"
```

---

## Task 4: Pass Tauri resource context through Listen and commands

**Files:**

- Modify: `apps/native/src-tauri/src/lib.rs`
- Modify: `apps/native/src-tauri/src/listen.rs`
- Modify: `apps/native/src-tauri/src/stt/whisper.rs`
- Modify: `apps/native/src/lib/commands.ts`
- Test: Rust command/status serialization and Listen setup tests

**Interfaces:**

- `listen_start` obtains the Tauri resource directory and passes a safe
  bundled-binary candidate into `ListenService::start`.
- `WhisperProvider::new` gains a bundled path/source context without exposing
  Tauri types in the STT abstraction.
- `whisper_status` returns source-aware local status.
- Missing bundle/fallback remains a durable sanitized setup error rather than a
  panic.

- [ ] **Step 1: Write failing Listen/status tests.** Assert bundled binary
      availability reports ready status, missing bundled/fallback reports setup
      state, and Listen startup uses the bundled path before fallback paths.

- [ ] **Step 2: Run focused tests and verify failure.**

```bash
cd apps/native/src-tauri
cargo test listen -- --nocapture
cargo test stt::whisper -- --nocapture
cargo test tests:: -- --nocapture
```

Expected: FAIL until resource context is threaded through production
startup/status.

- [ ] **Step 3: Thread only path/source data across boundaries.** Keep
      `ListenService` independent of `AppHandle`; pass `Option<PathBuf>` or a
      small platform-neutral bundled-binary context from `lib.rs`. Avoid putting
      Tauri resource APIs in `stt/whisper.rs`.

- [ ] **Step 4: Update command DTOs and wrappers.** Add the safe
      source/availability fields to `WhisperStatus` in `commands.ts`, preserving
      existing installed-model/download fields. Update `VoiceSetup` consumers
      only where TypeScript requires it; do not expose the path through Listen
      events.

- [ ] **Step 5: Verify setup errors.** Ensure missing binary produces the
      existing sanitized message, while bundled presence removes the misleading
      “install whisper-cli” state. Keep model download independent from binary
      execution.

- [ ] **Step 6: Run checks and commit.**

```bash
cargo test
cargo clippy --all-targets --all-features -- -D warnings
cargo check
cd ../../..
bun x tsc --noEmit
bun run build
bun test --pass-with-no-tests
```

```bash
git add apps/native/src-tauri/src/lib.rs apps/native/src-tauri/src/listen.rs \
  apps/native/src-tauri/src/stt/whisper.rs apps/native/src/lib/commands.ts
 git commit -m "feat(native): expose bundled Whisper runtime status"
```

---

## Task 5: Update Voice/Listen UI and package validation copy

**Files:**

- Modify: `apps/native/src/components/prefs/VoiceSetup.tsx`
- Modify: `apps/native/src/components/ListenSection.tsx`
- Modify: `apps/native/src/lib/events.ts` only if status event types change
- Test: TypeScript/build and existing frontend command checks

- [ ] **Step 1: Write a pure status-label test/helper if the frontend harness
      supports it.** Assert the labels map as follows:

```text
Bundled source → “Bundled with Marvis”
Path/Homebrew/User source → “Custom whisper-cli detected”
No source → “Whisper CLI unavailable”
```

If no frontend test harness exists, keep the mapping in a small pure function
that is covered by TypeScript/build and manually verify the three states.

- [ ] **Step 2: Replace stale guidance.** Do not show “install whisper-cli” when
      `WhisperStatus.source === bundled`. Show development guidance only when
      source is unavailable. Keep model download cards and Hugging Face source
      text unchanged.

- [ ] **Step 3: Keep Listen setup copy safe.** Listen can say local Whisper is
      unavailable, but it must not display resource paths, binary URLs,
      architecture details, or process output in public Listen events.

- [ ] **Step 4: Run frontend checks.**

```bash
cd apps/native
bun x tsc --noEmit
bun run build
bun test --pass-with-no-tests
```

- [ ] **Step 5: Commit.**

```bash
git add apps/native/src/components/prefs/VoiceSetup.tsx \
  apps/native/src/components/ListenSection.tsx apps/native/src/lib/events.ts
 git commit -m "feat(native): show bundled Whisper CLI status"
```

---

## Task 6: Verify release packaging and development fallback behavior

**Files:**

- Modify: `.github/workflows/whisper-cli.yml` or packaging scripts only for
  verification failures.
- Test: CI artifacts, Tauri bundle inspection, Rust/frontend verification,
  manual macOS checks.

- [ ] **Step 1: Run the pinned CI build workflow.** Confirm v1.9.2 source
      checkout, build flags, target architectures, executable outputs, and
      SHA-256 files. The workflow must fail if either binary is absent,
      non-executable, or the architecture does not match.

- [ ] **Step 2: Stage a target artifact and build the Tauri app.** Inspect the
      resulting app bundle with `file`, verify the bundled binary exists and is
      executable, and confirm the target architecture matches the app.

- [ ] **Step 3: Run automated verification.**

```bash
cd apps/native/src-tauri
cargo test
cargo clippy --all-targets --all-features -- -D warnings
cargo check
cd ../../..
bun x tsc --noEmit
bun run build
bun test --pass-with-no-tests
git diff --check
```

Expected: tests/clippy/check/typecheck/build pass. Existing unrelated `block
v0.1.6` future-incompatibility and Vite chunk warnings may remain documented.

- [ ] **Step 4: Run manual macOS matrix.**

1. Development build with no `whisper-cli` in PATH: bundled dev binary is
   preferred when staged.
2. Development build with no bundled binary: PATH/Homebrew/user-local fallback
   works.
3. Packaged arm64 `.app`: bundled arm64 binary is selected.
4. Packaged x86_64 artifact: bundled x86_64 binary is selected.
5. Voice Settings reports `Bundled with Marvis` for packaged builds.
6. Download Tiny, select it, and start Listen.
7. Confirm transcription launches the bundled binary.
8. Invalid/incompatible binary is rejected without launching or crashing.
9. Model updates preserve `~/.marvis/models/whisper/models/`.
10. No binary path, API key, URL, PCM, or transcript enters public Listen
    events.

- [ ] **Step 5: Document unavailable checks honestly.** If a release-signing or
      second-architecture environment is unavailable, mark it as not run with
      the exact reason; do not claim packaging verification passed.

- [ ] **Step 6: Commit only focused verification fixes.**

```bash
git add .github/workflows/whisper-cli.yml \
  apps/native/src-tauri/scripts/build-marvis.sh \
  apps/native/src-tauri/tauri.conf.json \
  apps/native/src-tauri/src/stt/whisper.rs \
  apps/native/src-tauri/src/voice_models.rs \
  apps/native/src-tauri/src/listen.rs apps/native/src-tauri/src/lib.rs \
  apps/native/src/lib/commands.ts \
  apps/native/src/components/prefs/VoiceSetup.tsx \
  apps/native/src/components/ListenSection.tsx
git commit -m "fix(native): harden bundled Whisper CLI verification findings"
```

---

## Coverage Check

- Pinned CI source build and checksums: Task 1.
- Tauri external binary/resource packaging: Task 2.
- Bundled-first/fallback runtime resolution: Task 3.
- Listen/AppState/Tauri resource context and status: Task 4.
- Settings/Listen source labels and safe copy: Task 5.
- Release packaging, architecture, fallback, and manual verification: Task 6.

No approved-spec requirement is intentionally unassigned.
