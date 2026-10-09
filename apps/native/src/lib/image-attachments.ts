/**
 * Ask composer image attachments — validation and JPEG normalization,
 * all in the webview. Accepted sources (JPEG, PNG, WebP) are decoded to
 * a bitmap, scaled so the longest edge fits `MAX_IMAGE_EDGE`, and
 * re-encoded as JPEG; the normalized payload is what `askSend` ships
 * and what Rust persists — the original file never leaves the browser.
 */
import type { AskImageInput } from './commands';

export type { AskImageInput };

/** A normalized image still waiting on the composer — `previewUrl` is
 *  the same JPEG as `jpegBase64`, wrapped as a data URL for the
 *  pending-chip thumbnail. `id` keys the chip: identical files
 *  normalize to identical payloads, so neither `previewUrl` nor `name`
 *  is a safe React key. */
export interface PendingAskImage extends AskImageInput {
  id: string;
  previewUrl: string;
}

let pendingSeq = 0;

export const MAX_ASK_ATTACHMENTS = 4;
export const MAX_IMAGE_SOURCE_BYTES = 20 * 1024 * 1024;
export const MAX_IMAGE_EDGE = 2048;

const SUPPORTED_IMAGE_TYPES = new Set([
  'image/jpeg',
  'image/png',
  'image/webp',
]);

/** `null` when the file can join the pending set; otherwise the
 *  user-facing rejection reason (toast-safe, no file contents). */
export const validateImageFile = (
  file: Pick<File, 'name' | 'type' | 'size'>,
  existingCount: number,
): string | null => {
  if (!SUPPORTED_IMAGE_TYPES.has(file.type)) {
    return 'Only JPEG, PNG, or WebP images can be attached';
  }
  if (file.size > MAX_IMAGE_SOURCE_BYTES) {
    return `“${file.name}” is over the 20 MiB per-image limit`;
  }
  if (existingCount >= MAX_ASK_ATTACHMENTS) {
    return `At most four images can be attached to one message`;
  }
  return null;
};

const jpegPayload = (dataUrl: string) => {
  const comma = dataUrl.indexOf(',');
  if (comma < 0) throw new Error('The image could not be normalized');
  return dataUrl.slice(comma + 1);
};

/** Decode → bound → re-encode as JPEG. Rejects with the same
 *  user-facing strings `validateImageFile` returns. */
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
    id: `pending-${pendingSeq++}`,
    name: file.name,
    jpegBase64: jpegPayload(previewUrl),
    previewUrl,
  };
};
