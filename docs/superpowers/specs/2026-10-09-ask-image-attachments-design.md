# Ask Image Attachments Design

**Date:** 2026-10-09

**Status:** Approved for implementation

## Goal

Allow Ask users to add up to four images through a native file picker or
drag-and-drop, send them to multimodal providers, and retain them as local
message attachments so retries and chat history remain useful after reload.

## Scope

### In scope

- Image picker and drag-and-drop in the Ask composer.
- JPEG, PNG, and WebP input.
- Browser-side normalization to JPEG.
- Four-image composer limit.
- Twenty MiB source-file limit per image.
- Longest-edge normalization to 2048px.
- Thumbnail chips with individual remove controls before send.
- Managed local JPEG persistence under `~/.marvis/attachments`.
- SQLite attachment metadata linked to user messages.
- Persisted thumbnails in Chat history after reload.
- Reuse of persisted images in Ask retries and subsequent conversation history.
- Failover that preserves user images and reports a clear error when no provider
  accepts multimodal input.

### Out of scope

- Arbitrary non-image file attachments.
- HEIC/HEIF support.
- Cloud upload or synchronization.
- Original-file preservation.
- Image OCR or local image understanding before provider submission.
- Image-only Ask sends; the typed question remains required.
- A new Tauri dialog or filesystem plugin.

## Existing context

The webview composer is split between `Bar.tsx` and
`components/bar/AskInput.tsx`. `askSend` currently accepts text, screen-read
intent, listen-session binding, and prompt-preset options. Ask history is
loaded from text-only `messages` rows by `ChatSection`.

The LLM core already has `ContentPart::ImageJpeg(Vec<u8>)`, and the OpenAI,
Anthropic, Gemini, Ollama, and OpenAI-compatible adapters already translate
image parts to their respective wire formats. Screen frames already use this
path. The feature extends the existing content model instead of creating a
second provider protocol.

The application already stores private local data under `~/.marvis`, and
Tauri's asset protocol currently scopes retained audio files. The existing
`base64` Rust dependency is sufficient for the normalized JPEG bridge payload.

## User flow

### Acquire and normalize

1. The user clicks the attachment button or drops files over the composer.
2. The webview rejects files that are not JPEG, PNG, or WebP, exceed 20 MiB,
   or exceed the four-image pending limit.
3. Each accepted file is decoded with the webview image APIs and drawn to a
   canvas whose longest edge is at most 2048px.
4. The canvas exports a JPEG preview/send payload. The original filename is
   kept as display metadata.
5. The composer renders a thumbnail chip with an individual remove control.

The normalizer returns a typed pending image containing the original name, a
preview data URL, and normalized JPEG base64. Pending images are frontend
state only until a send is accepted.

### Send

The text field remains required. On send, the frontend passes the normalized
payloads through the typed `askSend` wrapper. The command returns after
pre-flight; the streaming response continues through the existing `ask:*`
events.

The Rust Ask pipeline validates and persists the attachments before emitting
the loading boundary or contacting a provider. Once persistence succeeds, the
user row and its attachment metadata are available to the loading event,
`ask_current`, and `session_get`.

### History and retry

Chat history renders persisted user-message thumbnails using Tauri asset URLs.
`ask_retry` resolves the previous user row and loads its managed image bytes
into the same provider message shape used by the original send. Historical user
images are included when later Ask turns build conversation history.

## Type contracts

### Frontend input

```ts
export interface AskImageInput {
  name: string;
  jpegBase64: string;
}
```

`AskSendOpts` gains `attachments?: AskImageInput[]`. The command wrapper
always sends an array when attachments are present and otherwise sends an empty
array or omitted optional field according to the existing bridge convention.

### Persisted/output metadata

```ts
export interface MessageAttachment {
  id: number;
  name: string;
  path: string;
  mime: string;
  bytes: number;
}
```

`Message` gains `attachments: MessageAttachment[]`. Existing rows deserialize
with an empty array.

`ask:state` loading payloads and `ask_current` gain
`attachments: MessageAttachment[]` for the current user turn. The payload
contains managed metadata and paths only, never image bytes or base64.

The Rust command input mirrors the bridge naming rules with `jpeg_base64`; the
command boundary converts the frontend `jpegBase64` key to the Rust field.

## Persistence model

Add this table to the SQLite schema:

```sql
CREATE TABLE IF NOT EXISTS message_attachments (
    id         INTEGER PRIMARY KEY,
    message_id INTEGER NOT NULL,
    name       TEXT NOT NULL,
    path       TEXT NOT NULL,
    mime       TEXT NOT NULL,
    bytes      INTEGER NOT NULL,
    FOREIGN KEY (message_id) REFERENCES messages(id) ON DELETE CASCADE
);
```

Add a migration marker through the existing `storage::migrate` mechanism. The
`Message` query loads attachment metadata ordered by attachment id.

Files are written beneath `paths::attachments_dir()` using generated names
such as `attachment_<message_id>_<index>.jpg`; user-controlled names never
become filesystem paths. The attachment directory is created below the
existing `0700` Marvis root, and Unix files are written with private
permissions consistent with the rest of local user data.

Persistence sequencing:

1. Resolve the Ask session.
2. Insert the user message and obtain its id.
3. Decode and write each normalized JPEG to its generated path.
4. Insert the metadata rows.
5. On any failure, remove written files and delete the message before emitting
   an Ask error.

`message_delete` and `session_delete` collect managed paths before deleting
database rows, then remove those files best-effort. Text-only message deletion
remains unchanged.

## Ask pipeline changes

### Message construction

Extend `ChatMessage` with a helper that creates a user message containing
ordered text plus multiple `ImageJpeg` parts. `build_messages` receives:

- text/history context
- persisted historical image parts
- current user image parts
- optional screen frame
- optional screen description

History rows load image bytes from validated managed paths. The current user's
persisted images are passed to every provider candidate in the failover chain.

### Multimodal failover

The existing screen-frame path can retry without a frame when a provider
rejects screen images. User attachments are different: they are required user
input and must never be silently dropped.

For an attachment-bearing request:

- a multimodal rejection hands the same request to the next provider
  candidate;
- the request is not converted to text-only for that candidate;
- if all candidates reject or fail, `ask:error` reports a user-safe
  image-support message;
- the persisted user message and image metadata remain available for retry.

### Retry and resync

`ask_retry` reads the last user message plus its attachments. The loading event
includes the attachment metadata so a mounted Chat view can show the retried
user row immediately. `ask_current` includes the same metadata when the
webview mounts during an in-flight request.

## UI boundaries

### New frontend helper

Create `apps/native/src/lib/image-attachments.ts` for pure/DOM-bound image
normalization and validation. It owns:

- supported MIME checks
- source-size and count limits
- image decoding and JPEG normalization
- preview data URL creation
- filename/label handling

### New frontend component

Create `apps/native/src/components/bar/AskAttachments.tsx` for pending
thumbnail chips, remove controls, and the drop-zone visual state. It receives
state and callbacks; `Bar.tsx` owns the pending image list and send lifecycle.

### Existing components

- `AskInput.tsx` gains attachment-button, hidden-input, and drag/drop callbacks
  without owning attachment state.
- `Bar.tsx` collects pending images, passes them to `askSend`, and clears them
  after the command is accepted.
- `ChatSection.tsx` extends its message model and renders persisted
  user-message thumbnails with `convertFileSrc`.

Tauri IPC remains isolated to `commands.ts` and `events.ts`; React components
do not call `invoke` or raw `listen` directly.

## Backend file boundaries

### Modify

- `apps/native/src-tauri/src/paths.rs`
  - Add `attachments_dir()`.
- `apps/native/src-tauri/src/storage.rs`
  - Add schema/migration, attachment metadata, message loading, and cleanup
    helpers.
- `apps/native/src-tauri/src/ask.rs`
  - Accept attachment inputs, persist managed files, load historical image
    bytes, build multimodal messages, and preserve images during failover and
    retry.
- `apps/native/src-tauri/src/llm/mod.rs`
  - Add multiple-image user-message construction.
- `apps/native/src-tauri/src/lib.rs`
  - Extend `ask_send`, command registration, Ask resync payloads, and
    source-contract tests.
- `apps/native/src-tauri/tauri.conf.json`
  - Add `$HOME/.marvis/attachments/**` to the asset protocol scope.
- `apps/native/src/lib/commands.ts`
  - Add attachment input/output types and the `askSend` option.
- `apps/native/src/lib/events.ts`
  - Add attachment metadata to Ask loading/current contracts.
- `apps/native/src/components/bar/AskInput.tsx`
- `apps/native/src/views/Bar.tsx`
- `apps/native/src/components/ChatSection.tsx`

### Create

- `apps/native/src/lib/image-attachments.ts`
- `apps/native/src/lib/image-attachments.test.ts`
- `apps/native/src/components/bar/AskAttachments.tsx`

## Error handling and security

- Unsupported type, source-size overflow, decode failure, and pending-count
  overflow stay in the composer as user-visible validation errors.
- Base64 decode, JPEG validation, filesystem, and SQLite failures emit generic
  user-safe Ask errors; internal paths and raw error details are logged only
  where existing logging conventions permit.
- Managed paths are validated against the attachments directory before provider
  reads.
- Image bytes never appear in Ask events, logs, or persisted message text.
- The asset protocol scope includes only the managed attachment directory, not
  arbitrary user paths.
- Missing managed files prevent an attachment-dependent provider request rather
  than silently changing the question.
- No new dependency is required.

## Verification

Unit and integration tests must prove:

1. JPEG, PNG, and WebP files normalize successfully within the size/dimension
   limits.
2. Unsupported types, oversized inputs, and a fifth pending image are rejected.
3. Attachment metadata and bytes round-trip through SQLite plus managed files.
4. Message and session deletion removes attachment files.
5. Existing text-only message rows load with empty attachment arrays.
6. Ask loading/current/session-get payloads carry metadata without image bytes.
7. Historical and current image parts reach the provider in order.
8. Multimodal failover preserves user images and never silently drops them.
9. Retry reuses the persisted attachments.
10. Chat history renders persisted thumbnail assets.

Run:

```bash
cd apps/native
bun test
bun run check-types
cd src-tauri
cargo test
```

Also run the repository-level type check/build relevant to the changed packages
before declaring completion.

## Acceptance criteria

- A user can choose or drop up to four JPEG, PNG, or WebP images into Ask.
- The composer shows previews and supports individual removal before send.
- Sent images reach providers as multimodal content.
- Images remain available after reload, in Chat history, and through retry.
- Images are stored locally under the private Marvis data root and removed with
  their messages/sessions.
- Providers that do not support images do not receive a silent text-only
  request.
- Existing screen capture, text-only Ask, streaming, failover, and history
  behavior continue to pass verification.
