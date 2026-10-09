# Ask Image Attachments Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use
> `superpowers:executing-plans` to implement this plan task-by-task. Steps use
> checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add picker and drag/drop image attachments to Ask, persist normalized
JPEGs locally, render them in history, and preserve them through retries and
provider failover.

**Architecture:** Normalize JPEG, PNG, and WebP files in the webview, then send
only normalized JPEG base64 through the typed Ask command. Rust validates and
stores managed JPEGs under `~/.marvis/attachments`, records metadata in a
foreign-keyed SQLite table, and reconstructs `ImageJpeg` parts for current and
historical Ask messages. The existing provider adapters and screen-frame image
path remain the transport layer.

**Tech Stack:** React 19, TypeScript 6, Bun tests, Tauri 2, Rust 2021,
`rusqlite`, existing `base64` and JPEG-enabled `image` dependencies, existing
LLM provider adapters.

## Global Constraints

- Accept JPEG, PNG, and WebP only; HEIC/HEIF is out of scope.
- Allow at most four pending images and 20 MiB per source file.
- Normalize the longest edge to 2048px and persist/send JPEG output.
- The typed question remains required; image-only sends are not supported.
- Store image bytes under `~/.marvis/attachments`; do not upload or sync them.
- User attachments must never be silently removed for provider failover.
- React components and hooks use arrow-function syntax.
- Non-UI authored components use named exports only.
- Apps import icons through `@marvis/ui` with `Icon`-suffixed names.
- Tauri `invoke` and `listen` remain behind `commands.ts` and `events.ts`.
- Blocking filesystem work runs through `tauri::async_runtime::spawn_blocking`.
- Production code must be preceded by a failing test.
- Do not add a Tauri dialog or image-decoding dependency.

---

## File map

| File | Responsibility |
| --- | --- |
| `apps/native/src/lib/image-attachments.ts` | Browser validation and JPEG normalization |
| `apps/native/src/lib/image-attachments.test.ts` | Normalizer and limit tests |
| `apps/native/src/lib/commands.ts` | Ask input/output bridge types |
| `apps/native/src/lib/events.ts` | Ask loading/current attachment contracts |
| `apps/native/src/components/bar/AskAttachments.tsx` | Pending image chips and drop UI |
| `apps/native/src/components/bar/AskAttachments.test.tsx` | Pending-chip behavior |
| `apps/native/src/components/bar/AskInput.tsx` | Picker control and drag/drop callbacks |
| `apps/native/src/views/Bar.tsx` | Pending state and send integration |
| `apps/native/src/components/ChatSection.tsx` | Persisted history thumbnails and resync |
| `apps/native/src-tauri/src/paths.rs` | Managed attachment directory |
| `apps/native/src-tauri/src/storage.rs` | Attachment schema, metadata, cleanup |
| `apps/native/src-tauri/src/llm/mod.rs` | Multiple image content helper |
| `apps/native/src-tauri/src/ask.rs` | Persistence, history, retry, failover |
| `apps/native/src-tauri/src/lib.rs` | Tauri command and resync bridge |
| `apps/native/src-tauri/tauri.conf.json` | Managed asset scope |
| `packages/ui/src/index.ts` | Image/paperclip icon barrel exports |

---

### Task 1: Add image input types and the browser normalizer

**Files:**

- Create: `apps/native/src/lib/image-attachments.ts`
- Test: `apps/native/src/lib/image-attachments.test.ts`
- Modify: `apps/native/src/lib/commands.ts`

**Interfaces:**

```ts
export interface AskImageInput {
  name: string;
  jpegBase64: string;
}

export interface PendingAskImage extends AskImageInput {
  previewUrl: string;
}

export const MAX_ASK_ATTACHMENTS = 4;
export const MAX_IMAGE_SOURCE_BYTES = 20 * 1024 * 1024;
export const MAX_IMAGE_EDGE = 2048;

export const validateImageFile: (
  file: Pick<File, 'name' | 'type' | 'size'>,
  existingCount: number,
) => string | null;

export const normalizeImageFile: (
  file: File,
  existingCount: number,
) => Promise<PendingAskImage>;
```

- [ ] **Step 1: Add failing validation and normalization tests**

Create a test helper that stubs `globalThis.createImageBitmap` with an object
whose width is 4000, height is 2000, and whose `close` method is a no-op. Stub
`HTMLCanvasElement.prototype.getContext` and `toDataURL` so normalization is
deterministic.

```ts
test('rejects unsupported types, oversized files, and a fifth image', () => {
  const file = (type: string, size = 10) =>
    ({ name: 'input', type, size }) as File;

  expect(validateImageFile(file('image/gif'), 0)).toContain('JPEG');
  expect(
    validateImageFile(file('image/png', MAX_IMAGE_SOURCE_BYTES + 1), 0),
  ).toContain('20 MiB');
  expect(validateImageFile(file('image/png'), 4)).toContain('four');
  expect(validateImageFile(file('image/webp'), 3)).toBeNull();
});

test('normalizes an accepted image to a bounded JPEG payload', async () => {
  const image = await normalizeImageFile(
    new File([new Uint8Array([1])], 'diagram.png', { type: 'image/png' }),
    0,
  );

  expect(image.name).toBe('diagram.png');
  expect(image.previewUrl).toBe('data:image/jpeg;base64,PREVIEW');
  expect(image.jpegBase64).toBe('PREVIEW');
});
```

- [ ] **Step 2: Run the focused test and verify it fails**

```bash
cd apps/native
bun test src/lib/image-attachments.test.ts
```

Expected: FAIL because the types, constants, and normalizer do not exist.

- [ ] **Step 3: Add the command input type and normalizer implementation**

Add `AskImageInput` to the Ask section of `commands.ts`. In
`image-attachments.ts`, validate the MIME type, source size, and current count,
then decode and draw the image to a canvas:

```ts
const SUPPORTED_IMAGE_TYPES = new Set([
  'image/jpeg',
  'image/png',
  'image/webp',
]);

const jpegPayload = (dataUrl: string) => {
  const comma = dataUrl.indexOf(',');
  if (comma < 0) throw new Error('The image could not be normalized');
  return dataUrl.slice(comma + 1);
};

export const normalizeImageFile = async (
  file: File,
  existingCount: number,
): Promise<PendingAskImage> => {
  const error = validateImageFile(file, existingCount);
  if (error) throw new Error(error);
  const bitmap = await createImageBitmap(file);
  const scale = Math.min(
    1,
    MAX_IMAGE_EDGE / Math.max(bitmap.width, bitmap.height),
  );
  const canvas = document.createElement('canvas');
  canvas.width = Math.max(1, Math.round(bitmap.width * scale));
  canvas.height = Math.max(1, Math.round(bitmap.height * scale));
  const context = canvas.getContext('2d');
  if (!context) {
    bitmap.close();
    throw new Error('Images are unavailable in this window');
  }
  context.drawImage(bitmap, 0, 0, canvas.width, canvas.height);
  bitmap.close();
  const previewUrl = canvas.toDataURL('image/jpeg', 0.85);
  return {
    name: file.name,
    jpegBase64: jpegPayload(previewUrl),
    previewUrl,
  };
};
```

`validateImageFile` must return user-facing strings for unsupported MIME type,
source size overflow, and the fifth pending image. Do not log file contents.

- [ ] **Step 4: Run the focused test and verify it passes**

```bash
cd apps/native
bun test src/lib/image-attachments.test.ts
```

Expected: PASS with the existing test suite unaffected.

- [ ] **Step 5: Commit the normalizer**

```bash
git add apps/native/src/lib/commands.ts \
  apps/native/src/lib/image-attachments.ts \
  apps/native/src/lib/image-attachments.test.ts
git commit -m "feat: normalize Ask image attachments"
```

---

### Task 2: Add attachment controls and pending-image UI

**Files:**

- Create: `apps/native/src/components/bar/AskAttachments.tsx`
- Test: `apps/native/src/components/bar/AskAttachments.test.tsx`
- Modify: `apps/native/src/components/bar/AskInput.tsx`
- Modify: `apps/native/src/views/Bar.tsx`
- Modify: `packages/ui/src/index.ts`

**Interfaces:**

```ts
export interface AskAttachmentsProps {
  images: PendingAskImage[];
  error: string | null;
  onRemove: (index: number) => void;
}

export interface AskInputAttachmentProps {
  onPick: () => void;
  onFiles: (files: File[]) => void;
  dropActive: boolean;
}
```

- [ ] **Step 1: Add failing pending-chip tests**

Render `AskAttachments` with two `PendingAskImage` values and assert that both
preview URLs and names are present, each remove button calls the matching index,
and the validation error is visible. Use the same direct React DOM harness as
`SessionPlayer.test.tsx`; do not add a testing-library dependency.

```tsx
test('renders pending previews and removes one attachment', async () => {
  const removed: number[] = [];
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = createRoot(host);
  await act(async () =>
    root.render(
      <AskAttachments
        images={[
          {
            name: 'one.png',
            jpegBase64: 'ONE',
            previewUrl: 'data:image/jpeg;base64,ONE',
          },
          {
            name: 'two.webp',
            jpegBase64: 'TWO',
            previewUrl: 'data:image/jpeg;base64,TWO',
          },
        ]}
        error=''
        onRemove={(index) => removed.push(index)}
      />,
    ),
  );

  expect(host.querySelectorAll('img')).toHaveLength(2);
  expect(host.textContent).toContain('one.png');
  (host.querySelector('[aria-label="Remove two.webp"]') as HTMLButtonElement)
    .click();
  expect(removed).toEqual([1]);

  await act(async () => root.unmount());
  host.remove();
});
```

- [ ] **Step 2: Run the focused UI test and verify it fails**

```bash
cd apps/native
bun test src/components/bar/AskAttachments.test.tsx
```

Expected: FAIL because the component does not exist.

- [ ] **Step 3: Export image attachment icons through `@marvis/ui`**

Add `ImageIcon` and `PaperclipIcon` to the existing lucide export list in
`packages/ui/src/index.ts`. Keep the `Icon` suffix and do not import directly
from `lucide-react` in the native app.

- [ ] **Step 4: Implement the pending-image component**

Render a compact row of `<img>` thumbnails with accessible remove buttons. The
component must render nothing when `images` is empty and must render `error`
when it is non-empty. Keep the component state-free; pending images stay in
`Bar.tsx`.

- [ ] **Step 5: Add picker and drag/drop props to `AskInput`**

Add a hidden file input with:

```tsx
<input
  ref={fileInputRef}
  type='file'
  accept='image/jpeg,image/png,image/webp'
  multiple
  className='sr-only'
  onChange={(event) => {
    onFiles(Array.from(event.currentTarget.files ?? []));
    event.currentTarget.value = '';
  }}
/>
```

Add an attachment button using `PaperclipIcon`. Its click handler calls
`fileInputRef.current?.click()`. On the composer wrapper, prevent the browser's
default drop behavior, set `dropEffect = 'copy'`, and pass dropped image files to
`onFiles`. Stop propagation so the bar drag-region handler cannot consume the
drop.

- [ ] **Step 6: Own pending state and normalization in `Bar.tsx`**

Add:

```ts
const [pendingImages, setPendingImages] = useState<PendingAskImage[]>([]);
const [attachmentError, setAttachmentError] = useState<string | null>(null);

const addFiles = async (files: File[]) => {
  setAttachmentError(null);
  const next = [...pendingImages];
  try {
    for (const file of files) {
      next.push(await normalizeImageFile(file, next.length));
    }
    setPendingImages(next);
  } catch (error) {
    setAttachmentError(
      error instanceof Error ? error.message : 'Could not add image',
    );
  }
};
```

Use the functional form of `setPendingImages` if multiple drop events can land
in the same tick. Render `AskAttachments` above the input row, pass the pending
images to `AskInput`, and remove by index. Keep text required in `sendAsk`.

- [ ] **Step 7: Run frontend tests and type checks**

```bash
cd apps/native
bun test src/lib/image-attachments.test.ts \
  src/components/bar/AskAttachments.test.tsx
bun run check-types
```

Expected: PASS.

- [ ] **Step 8: Commit the composer UI**

```bash
git add packages/ui/src/index.ts \
  apps/native/src/components/bar/AskAttachments.tsx \
  apps/native/src/components/bar/AskAttachments.test.tsx \
  apps/native/src/components/bar/AskInput.tsx \
  apps/native/src/views/Bar.tsx
git commit -m "feat: add Ask image picker and drop UI"
```

---

### Task 3: Extend the typed Ask and event contracts

**Files:**

- Modify: `apps/native/src/lib/commands.ts`
- Modify: `apps/native/src/lib/events.ts`
- Modify: `apps/native/src-tauri/src/lib.rs`

**Interfaces:**

```ts
export interface MessageAttachment {
  id: number;
  name: string;
  path: string;
  mime: string;
  bytes: number;
}

export interface AskSendOpts {
  withScreen?: boolean;
  listenId?: number;
  presetId?: string;
  presetLang?: string;
  attachments?: AskImageInput[];
}
```

- [ ] **Step 1: Add a failing bridge contract test**

Add a Rust source-contract test beside the existing `ask_send` registration
assertions. It must assert that `ask_send` has an `attachments` parameter, that
`ask_send` remains registered, and that the event/current payload source
contains `attachments`.

```rust
#[test]
fn ask_attachment_contract_is_registered() {
    let source = include_str!("lib.rs");
    let body = source
        .split("fn ask_send(")
        .nth(1)
        .and_then(|rest| rest.split("\n}").next())
        .expect("ask_send body not found");
    assert!(body.contains("attachments"));
    assert!(source.contains("ask_send,"));
}
```

- [ ] **Step 2: Run the contract test and verify it fails**

```bash
cd apps/native/src-tauri
cargo test ask_attachment_contract_is_registered
```

Expected: FAIL because the command does not accept attachments yet.

- [ ] **Step 3: Add TypeScript types and pass attachments through `askSend`**

Extend `AskSendOpts` and change the invoke payload to:

```ts
export const askSend = (text: string, opts: AskSendOpts = {}) =>
  invoke<void>('ask_send', {
    text,
    withScreen: opts.withScreen ?? false,
    listenId: opts.listenId,
    presetId: opts.presetId,
    presetLang: opts.presetLang,
    attachments: opts.attachments ?? [],
  });
```

Add `attachments: MessageAttachment[]` to `Message` and to the Ask loading/current
payload types, defaulting to an empty array for existing callers.

- [ ] **Step 4: Add the Rust command input and event fields**

In `lib.rs`, define:

```rust
#[derive(Debug, Clone, serde::Deserialize)]
pub struct AskAttachmentInput {
    pub name: String,
    pub jpeg_base64: String,
}
```

Add `attachments: Option<Vec<AskAttachmentInput>>` to `ask_send`, pass
`unwrap_or_default()` into `AskService::send`, and keep the existing gate guard.
Update `ask_current` serialization to include an empty/current attachment list.
Update `events.ts` comments/interfaces to document the same payload shape.

- [ ] **Step 5: Run contract and TypeScript verification**

```bash
cd apps/native/src-tauri
cargo test ask_attachment_contract_is_registered
cd ../..
bun run check-types
```

Expected: PASS.

- [ ] **Step 6: Commit the bridge contract**

```bash
git add apps/native/src/lib/commands.ts \
  apps/native/src/lib/events.ts \
  apps/native/src-tauri/src/lib.rs
git commit -m "feat: extend Ask contract for image attachments"
```

---

### Task 4: Add managed attachment storage and cleanup

**Files:**

- Modify: `apps/native/src-tauri/src/paths.rs`
- Modify: `apps/native/src-tauri/src/storage.rs`
- Test: `apps/native/src-tauri/src/storage.rs`

**Interfaces:**

```rust
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct MessageAttachment {
    pub id: i64,
    pub message_id: i64,
    pub name: String,
    pub path: String,
    pub mime: String,
    pub bytes: i64,
}

pub fn attachments_dir() -> PathBuf;

pub fn message_attachment_add(
    &self,
    message_id: i64,
    name: &str,
    path: &str,
    mime: &str,
    bytes: i64,
) -> anyhow::Result<i64>;

pub fn message_attachments(
    &self,
    message_id: i64,
) -> anyhow::Result<Vec<MessageAttachment>>;
```

- [ ] **Step 1: Add failing storage tests**

Add a migration/round-trip test using the existing `tmp_dir()` helper:

```rust
#[test]
fn message_attachments_round_trip_and_delete_with_session() {
    let dir = tmp_dir();
    let db = Db::at(dir.join("marvis.db")).unwrap();
    let session = db.session_get_or_create_active("ask").unwrap();
    let message = db.message_add(session, "user", "look at this").unwrap();
    let image = dir.join("attachment.jpg");
    std::fs::write(&image, [1_u8, 2, 3]).unwrap();

    db.message_attachment_add(
        message,
        "photo.png",
        image.to_str().unwrap(),
        "image/jpeg",
        3,
    )
    .unwrap();
    let rows = db.messages_for(session).unwrap();
    assert_eq!(rows[0].attachments.len(), 1);
    assert_eq!(rows[0].attachments[0].name, "photo.png");

    db.session_delete(session).unwrap();
    assert!(!image.exists());
    let _ = std::fs::remove_dir_all(&dir);
}
```

- [ ] **Step 2: Run the storage test and verify it fails**

```bash
cd apps/native/src-tauri
cargo test message_attachments_round_trip_and_delete_with_session
```

Expected: FAIL because the table, struct field, and methods do not exist.

- [ ] **Step 3: Add the managed directory helper**

Add this to `paths.rs` without creating the directory eagerly:

```rust
pub fn attachments_dir() -> PathBuf {
    root().join("attachments")
}
```

Extend the existing path test to assert that it resolves to
`root().join("attachments")` and does not create the directory by itself.

- [ ] **Step 4: Add the SQLite table and attachment metadata**

Add `message_attachments` to `SCHEMA` with `message_id` foreign-key cascade.
Add a migration marker through the existing `migrate` mechanism. Extend
`storage::Message` with `attachments: Vec<MessageAttachment>` and load the
ordered child rows in `messages_for`.

Implement `message_attachment_add` and `message_attachments`. Update
`session_delete` to collect attachment paths before deleting the session, then
remove them best-effort. Update `message_delete` similarly so retry cleanup
removes managed files.

Do not change the existing message ordering or metadata columns.

- [ ] **Step 5: Run the storage tests and verify they pass**

```bash
cd apps/native/src-tauri
cargo test storage::tests
```

Expected: PASS, including all pre-existing storage migration, ordering, and
session-deletion tests.

- [ ] **Step 6: Commit managed storage**

```bash
git add apps/native/src-tauri/src/paths.rs \
  apps/native/src-tauri/src/storage.rs
git commit -m "feat: persist Ask message attachments locally"
```

---

### Task 5: Extend the LLM message model for multiple images

**Files:**

- Modify: `apps/native/src-tauri/src/llm/mod.rs`
- Modify: `apps/native/src-tauri/src/ask.rs`
- Test: `apps/native/src-tauri/src/ask.rs`

**Interfaces:**

```rust
impl ChatMessage {
    pub fn user_with_images(
        text: impl Into<String>,
        images: Vec<Vec<u8>>,
    ) -> Self;
}
```

- [ ] **Step 1: Add a failing multiple-image message test**

Add to the Ask test module:

```rust
#[test]
fn build_messages_keeps_all_user_images_after_text() {
    let messages = build_messages(
        &[],
        "",
        "describe these",
        &[vec![1, 2], vec![3, 4]],
        None,
        None,
        "en",
        None,
    );
    let user = messages.last().unwrap();
    assert_eq!(user.role, Role::User);
    assert_eq!(
        user.content,
        vec![
            ContentPart::Text("describe these".into()),
            ContentPart::ImageJpeg(vec![1, 2]),
            ContentPart::ImageJpeg(vec![3, 4]),
        ],
    );
}
```

Update the test call with the exact final `build_messages` argument order while
keeping the assertion unchanged.

- [ ] **Step 2: Run the test and verify it fails**

```bash
cd apps/native/src-tauri
cargo test build_messages_keeps_all_user_images_after_text
```

Expected: FAIL because `build_messages` has no image-list argument and
`user_with_images` does not exist.

- [ ] **Step 3: Implement the content helper and builder changes**

Implement `user_with_images` by creating a text part followed by one
`ImageJpeg` part per input. Update `build_messages` to receive
`images: &[Vec<u8>]`; append current user images before the optional screen
frame. Keep historical messages text-only until Task 7 loads their images.

Update every existing `build_messages` test call with `&[]` so text-only and
screen-frame tests retain their current behavior.

- [ ] **Step 4: Run provider and Ask tests**

```bash
cd apps/native/src-tauri
cargo test llm::
cargo test ask::tests::build_messages_keeps_all_user_images_after_text
```

Expected: PASS. Existing OpenAI, Anthropic, Gemini, Ollama, and compatible
request-body tests must remain green because they already iterate all content
parts.

- [ ] **Step 5: Commit the message-model change**

```bash
git add apps/native/src-tauri/src/llm/mod.rs \
  apps/native/src-tauri/src/ask.rs
git commit -m "feat: build Ask messages with multiple images"
```

---

### Task 6: Persist current-send images in the Ask pipeline

**Files:**

- Modify: `apps/native/src-tauri/src/ask.rs`
- Modify: `apps/native/src-tauri/src/lib.rs`
- Test: `apps/native/src-tauri/src/ask.rs`

**Interfaces:**

```rust
pub struct AskAttachmentInput {
    pub name: String,
    pub jpeg_base64: String,
}

pub struct SendOpts<'a> {
    pub text: &'a str,
    pub with_screen: bool,
    pub screen_required: bool,
    pub regenerate: bool,
    pub listen_id: Option<i64>,
    pub preset: Option<String>,
    pub preset_lang: Option<String>,
    pub attachments: Vec<AskAttachmentInput>,
}

pub struct ChainOpts<'a> {
    pub text: &'a str,
    pub fresh_session: bool,
    pub regenerate: bool,
    pub listen_id: Option<i64>,
    pub language: &'a str,
    pub instruction: Option<&'a str>,
    pub preset_id: Option<&'a str>,
    pub attachments: Vec<AskAttachmentInput>,
}
```

- [ ] **Step 1: Add failing persistence and provider-input tests**

Add a `send_chain` test with two base64-encoded JPEG payloads. Use the existing
mock provider and assert both that the user message has two metadata rows and
that the provider receives two `ContentPart::ImageJpeg` parts:

```rust
fn test_jpeg_base64() -> String {
    use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
    use std::io::Cursor;

    let image = image::RgbImage::from_pixel(1, 1, image::Rgb([255, 255, 255]));
    let mut output = Cursor::new(Vec::new());
    image::DynamicImage::ImageRgb8(image)
        .write_to(&mut output, image::ImageFormat::Jpeg)
        .unwrap();
    B64.encode(output.into_inner())
}

#[tokio::test]
async fn send_chain_persists_and_sends_current_images() {
    let dir = tmp_dir();
    let db = Arc::new(Db::at(dir.join("marvis.db")).unwrap());
    let jpeg = test_jpeg_base64();
    let provider = MockProvider::new(vec![Behavior::Tokens(vec!["ok".into()])]);
    let calls = provider.calls();
    let (events, emit) = recorder();
    let reader = screen_read::ScreenReader::new();
    let ring = Mutex::new(RingBuffer::new(4, 1 << 20));
    let cancel = CancellationToken::new();

    send_chain(
        Arc::clone(&db),
        vec![candidate("mock", provider)],
        None,
        &emit,
        &input(&reader, &ring),
        &cancel,
        ChainOpts {
            text: "what is this?",
            attachments: vec![
                AskAttachmentInput {
                    name: "one.png".into(),
                    jpeg_base64: jpeg.clone(),
                },
                AskAttachmentInput {
                    name: "two.webp".into(),
                    jpeg_base64: jpeg,
                },
            ],
            language: "en",
            ..ChainOpts::default()
        },
    )
    .await
    .unwrap();

    let session = db.session_active_id("ask").unwrap().unwrap();
    assert_eq!(db.messages_for(session).unwrap()[0].attachments.len(), 2);
    let user = &calls.lock()[0][1];
    assert_eq!(
        user.content
            .iter()
            .filter(|part| matches!(part, ContentPart::ImageJpeg(_)))
            .count(),
        2,
    );
    assert!(events.lock().iter().any(|(name, payload)|
        name == EV_STATE && payload["state"] == "loading"));
    let _ = std::fs::remove_dir_all(&dir);
}
```

- [ ] **Step 2: Run the test and verify it fails**

```bash
cd apps/native/src-tauri
cargo test send_chain_persists_and_sends_current_images
```

Expected: FAIL because `send_chain` does not accept `Arc<Db>`/attachments and no
persistence path exists.

- [ ] **Step 3: Add backend validation and managed-file preparation**

Implement a blocking helper with this contract:

```rust
fn persist_user_message_with_attachments(
    db: &Db,
    session_id: i64,
    text: &str,
    preset: Option<&str>,
    attachments: &[AskAttachmentInput],
) -> Result<Vec<MessageAttachment>, String>;
```

The helper must:

1. Reject more than four inputs.
2. Decode standard base64.
3. Reject decoded data over 20 MiB.
4. Validate each payload as JPEG using the existing JPEG-enabled `image`
   dependency.
5. Insert the user message.
6. Create `paths::attachments_dir()` and write generated private paths.
7. Insert metadata rows.
8. Remove written files and the message on any failure.

Sanitize only the stored display name: remove controls, cap it at 80 Unicode
scalars, and use `attachment_<message_id>_<index>.jpg` for the actual path.
Never include paths or base64 data in an emitted event or user-facing error.

- [ ] **Step 4: Run persistence tests and fix only production code**

```bash
cd apps/native/src-tauri
cargo test send_chain_persists_and_sends_current_images
cargo test storage::tests
```

Expected: PASS. If the test fails, correct backend code rather than weakening
the assertions.

- [ ] **Step 5: Thread attachments through `AskService::send` and `send_chain`**

Change `AskService::send` and `SendOpts` to carry the input slice/vector. Keep
`ask_send` gate behavior unchanged. Resolve the Ask session before persistence,
and run the filesystem/database helper inside
`tauri::async_runtime::spawn_blocking` using an `Arc<Db>` captured by the task.

Only emit the normal loading event after attachment persistence succeeds. If
persistence fails, emit the existing error-plus-idle sequence and do not call a
provider. Text-only sends retain their current best-effort message persistence.

- [ ] **Step 6: Run the full Ask Rust tests**

```bash
cd apps/native/src-tauri
cargo test ask::tests
```

Expected: PASS, including existing streaming, cancellation, title, failover,
and screen-material tests.

- [ ] **Step 7: Commit current-send persistence**

```bash
git add apps/native/src-tauri/src/ask.rs \
  apps/native/src-tauri/src/lib.rs
git commit -m "feat: persist current Ask image attachments"
```

---

### Task 7: Load historical images and preserve them through retry/failover

**Files:**

- Modify: `apps/native/src-tauri/src/ask.rs`
- Modify: `apps/native/src-tauri/src/storage.rs` if query helpers need adjustment
- Test: `apps/native/src-tauri/src/ask.rs`

**Interfaces:**

```rust
fn rows_to_history(
    rows: &[crate::storage::Message],
) -> Result<Vec<ChatMessage>, String>;

fn rows_to_history_at(
    rows: &[crate::storage::Message],
    attachments_root: &std::path::Path,
) -> Result<Vec<ChatMessage>, String>;

fn read_managed_attachment(
    attachment: &MessageAttachment,
) -> Result<Vec<u8>, String>;
```

- [ ] **Step 1: Add failing history and multimodal failover tests**

Add tests for these behaviors:

```rust
#[tokio::test]
async fn multimodal_failover_keeps_images_for_the_next_provider() {
    let dir = tmp_dir();
    let db = Arc::new(Db::at(dir.join("marvis.db")).unwrap());
    let first = MockProvider::new(vec![Behavior::Fail(
        LlmError::MultimodalUnsupported,
    )]);
    let second = MockProvider::new(vec![Behavior::Tokens(vec!["ok".into()])]);
    let second_calls = second.calls();
    let (_events, emit) = recorder();
    let reader = screen_read::ScreenReader::new();
    let ring = Mutex::new(RingBuffer::new(4, 1 << 20));
    let cancel = CancellationToken::new();

    send_chain(
        Arc::clone(&db),
        vec![candidate("first", first), candidate("second", second)],
        None,
        &emit,
        &input(&reader, &ring),
        &cancel,
        ChainOpts {
            text: "q",
            attachments: vec![AskAttachmentInput {
                name: "photo.jpg".into(),
                jpeg_base64: test_jpeg_base64(),
            }],
            language: "en",
            ..ChainOpts::default()
        },
    )
    .await
    .unwrap();

    let second_request = &second_calls.lock()[0];
    assert_eq!(
        second_request[1]
            .content
            .iter()
            .filter(|part| matches!(part, ContentPart::ImageJpeg(_)))
            .count(),
        1,
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn rows_to_history_loads_persisted_user_images() {
    let dir = tmp_dir();
    let attachments_root = dir.join("attachments");
    std::fs::create_dir_all(&attachments_root).unwrap();
    let path = attachments_root.join("attachment_1_0.jpg");
    std::fs::write(&path, [1_u8, 2, 3]).unwrap();
    let rows = vec![crate::storage::Message {
        id: 1,
        session_id: 1,
        role: "user".into(),
        content: "see this".into(),
        provider: None,
        model: None,
        tokens_in: None,
        tokens_out: None,
        preset: None,
        ts: 1,
        attachments: vec![MessageAttachment {
            id: 1,
            message_id: 1,
            name: "photo.jpg".into(),
            path: path.to_string_lossy().into_owned(),
            mime: "image/jpeg".into(),
            bytes: 3,
        }],
    }];

    let history = rows_to_history_at(&rows, &attachments_root).unwrap();
    assert!(matches!(
        &history[0].content[1],
        ContentPart::ImageJpeg(bytes) if bytes == &vec![1, 2, 3]
    ));
    let _ = std::fs::remove_dir_all(&dir);
}
```

Reuse the `test_jpeg_base64()` helper from Task 6. The two tests must call the
actual `rows_to_history_at` and `stream_candidate` paths, not test doubles.

- [ ] **Step 2: Run the tests and verify they fail**

```bash
cd apps/native/src-tauri
cargo test multimodal_failover_keeps_images_for_the_next_provider
cargo test rows_to_history_loads_persisted_user_images
```

Expected: FAIL because history currently drops image metadata and multimodal
retry currently removes the frame without distinguishing user attachments.

- [ ] **Step 3: Validate and load managed attachment paths**

Implement `read_managed_attachment` by canonicalizing the configured
attachments directory and the metadata path, requiring the file to remain under
the managed directory, then reading it. Return a user-safe error if the file is
missing or outside the directory.

Update `rows_to_history` so user rows use text-only `ChatMessage::text` when
there are no attachments and `ChatMessage::user_with_images` when attachments
exist. Assistant rows remain text-only.

- [ ] **Step 4: Separate screen fallback from required user images**

Extend `stream_candidate` with `user_images: &[Vec<u8>]`. Build the request with
both user images and the optional screen frame. Change the multimodal retry rule
to:

```rust
if !retried
    && user_images.is_empty()
    && frame.is_some()
    && error.is_multimodal()
{
    retried = true;
    // Retry without only the optional screen frame.
    continue;
}
```

When `user_images` is non-empty, return the provider failure to the chain so the
next candidate receives the complete image-bearing request. Do not construct a
text-only retry for that request.

- [ ] **Step 5: Update retry and current Ask state**

Make `ask_retry` resolve the last user message's attachment metadata and pass
those bytes into the same `ChainOpts` path as a new send. Add
`current_attachments: Mutex<Vec<MessageAttachment>>` to `AskService`; clear it
on a new run/close, set it from the persisted user row after loading, and include
it in `current_payload`.

Ensure loading events carry metadata on both a new send and retry. Existing
`ask_current` consumers must receive `attachments: []` for text-only runs.

- [ ] **Step 6: Run Ask and provider tests**

```bash
cd apps/native/src-tauri
cargo test ask::tests
cargo test llm::
```

Expected: PASS. Specifically verify that a multimodal rejection does not drop
images, screen-only fallback still works, and text-only requests remain
unchanged.

- [ ] **Step 7: Commit history and failover behavior**

```bash
git add apps/native/src-tauri/src/ask.rs \
  apps/native/src-tauri/src/storage.rs
git commit -m "feat: preserve Ask images through history and retry"
```

---

### Task 8: Render persisted attachments in Chat history and resync

**Files:**

- Modify: `apps/native/src/components/ChatSection.tsx`
- Modify: `apps/native/src/lib/commands.ts`
- Modify: `apps/native/src/lib/events.ts`
- Test: `apps/native/src/components/ChatSection.test.tsx`
- Modify: `apps/native/src-tauri/tauri.conf.json`

**Interfaces:**

```ts
interface ChatMsg {
  role: 'user' | 'assistant';
  content: string;
  attachments: MessageAttachment[];
  ts?: number;
  preset?: string | null;
  provider?: string | null;
  model?: string | null;
  tokensIn?: number | null;
  tokensOut?: number | null;
}
```

- [ ] **Step 1: Add a failing Chat history thumbnail test**

Create a direct React DOM test using the existing `happy-dom` harness. Mock the
command/event modules before dynamically importing `ChatSection`, provide one
active session row with one attachment, and provide a deterministic
`convertFileSrc` implementation:

```tsx
import { expect, mock, test } from 'bun:test';
import { GlobalWindow } from 'happy-dom';
import { act } from 'react';
import { createRoot } from 'react-dom/client';

const row = {
  id: 4,
  session_id: 1,
  role: 'user',
  content: 'look at this',
  provider: null,
  model: null,
  tokens_in: null,
  tokens_out: null,
  preset: null,
  ts: 1,
  attachments: [{
    id: 9,
    message_id: 4,
    name: 'diagram.png',
    path: '/managed/diagram.jpg',
    mime: 'image/jpeg',
    bytes: 3,
  }],
};

mock.module('@/lib/commands', () => ({
  askCurrent: async () => ({
    state: 'idle',
    question: '',
    response: '',
    error: null,
    attachments: [],
  }),
  askRetry: async () => {},
  presetsList: async () => [],
  sessionEndActive: async () => {},
  sessionGet: async () => [row],
  sessionList: async () => [{
    id: 1,
    kind: 'ask',
    title: null,
    audio_file: null,
    stt: null,
    started_at: 1,
    ended_at: null,
    last_active_at: 1,
  }],
}));
mock.module('@/lib/events', () => ({
  EV_ASK_CHUNK: 'ask:chunk',
  EV_ASK_DONE: 'ask:done',
  EV_ASK_ERROR: 'ask:error',
  EV_ASK_STATE: 'ask:state',
  EV_CONFIG_CHANGED: 'config:changed',
  useTauriEvent: () => {},
}));

const { ChatSection } = await import('./ChatSection');

test('renders persisted user attachment thumbnails', async () => {
  const win = new GlobalWindow();
  Object.assign(globalThis, {
    window: win,
    document: win.document,
    IS_REACT_ACT_ENVIRONMENT: true,
  });
  Object.assign(win, {
    __TAURI_INTERNALS__: {
      convertFileSrc: (path: string) => `asset://${path}`,
    },
  });
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = createRoot(host);

  await act(async () => root.render(<ChatSection onBack={() => {}} />));
  await act(async () => Promise.resolve());

  const image = host.querySelector('img');
  expect(image?.getAttribute('src')).toBe('asset:///managed/diagram.jpg');
  expect(image?.getAttribute('alt')).toBe('diagram.png');

  await act(async () => root.unmount());
  host.remove();
  await win.happyDOM.close();
});
```

- [ ] **Step 2: Run the test and verify it fails**

```bash
cd apps/native
bun test src/components/ChatSection.test.tsx
```

Expected: FAIL because `ChatMsg` does not have attachments and Chat history
renders text-only user bubbles.

- [ ] **Step 3: Extend command/event types and Chat message folding**

Add `attachments: MessageAttachment[]` to `Message`, `AskCurrent`, and the
loading event payload. Update `rowsToMsgs`, `applyLoading`, `setTail` call sites,
and mount resync to preserve attachment arrays. Existing rows use `[]`.

- [ ] **Step 4: Render safe asset URLs**

Import `convertFileSrc` from `@tauri-apps/api/core` and render thumbnails only
for user messages:

```tsx
{m.attachments.map((attachment) => (
  <img
    key={attachment.id}
    src={convertFileSrc(attachment.path)}
    alt={attachment.name}
    className='max-h-40 max-w-56 rounded-lg object-contain'
  />
))}
```

Keep the existing typed IPC boundary; `convertFileSrc` is a scoped Tauri core
API, not a command or event subscription.

- [ ] **Step 5: Expand the Tauri asset scope**

In `tauri.conf.json`, change the asset protocol scope to include both:

```json
"scope": [
  "$HOME/.marvis/audios/**",
  "$HOME/.marvis/attachments/**"
]
```

Do not broaden the scope to all of `$HOME`.

- [ ] **Step 6: Run frontend tests and type checks**

```bash
cd apps/native
bun test src/components/ChatSection.test.tsx
bun run check-types
```

Expected: PASS.

- [ ] **Step 7: Commit Chat history rendering**

```bash
git add apps/native/src/components/ChatSection.tsx \
  apps/native/src/components/ChatSection.test.tsx \
  apps/native/src/lib/commands.ts \
  apps/native/src/lib/events.ts \
  apps/native/src-tauri/tauri.conf.json
git commit -m "feat: render persisted Ask image attachments"
```

---

### Task 9: End-to-end verification and cleanup review

**Files:**

- Review all files changed in Tasks 1–8.
- Update only tests or implementation files required by failures.

- [ ] **Step 1: Run the complete native frontend suite**

```bash
cd apps/native
bun test
bun run check-types
```

Expected: PASS with no new warnings or errors.

- [ ] **Step 2: Run the complete Rust suite**

```bash
cd apps/native/src-tauri
cargo test
```

Expected: PASS, including storage migrations, Ask streaming, provider adapter,
and command-contract tests.

- [ ] **Step 3: Run the repository-level checks**

```bash
cd ../../..
bun run check-types
bun run build
```

Expected: successful native, UI, and web workspace checks/builds.

- [ ] **Step 4: Perform manual QA**

Run the native app and verify:

1. The paperclip control opens the native file picker.
2. JPEG, PNG, and WebP files show normalized previews.
3. Dragging images over the composer shows the drop state.
4. A fifth image and oversized/unsupported files show validation errors.
5. Removing a pending image changes the send payload.
6. A sent user message renders its thumbnails immediately.
7. Reloading Chat renders thumbnails from managed asset paths.
8. Retry sends the same images again.
9. A text-only provider failure can fail over without losing images.
10. If all providers reject images, the user sees an error and the message
    remains retryable.
11. Deleting the session removes attachment files from the managed directory.

- [ ] **Step 5: Review the final diff for security and scope**

Run:

```bash
git diff develop...HEAD --stat
git diff develop...HEAD --check
git status --short
```

Confirm that no image bytes, provider keys, or unrelated files are logged or
committed, that no new dependency was added, and that the diff contains only
the two approved Phase 1 features plus their tests and contracts.

- [ ] **Step 6: Commit any final test-driven fix**

If verification finds a defect, first add the smallest failing regression test,
then implement the fix, rerun the affected suite and the full verification
commands, and commit the fix with a reason-focused message.
