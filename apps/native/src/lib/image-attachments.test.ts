/// <reference types="bun-types" />
import { expect, test } from 'bun:test';
import { GlobalWindow } from 'happy-dom';
import {
  MAX_IMAGE_SOURCE_BYTES,
  normalizeImageFile,
  validateImageFile,
} from './image-attachments';

const file = (type: string, size = 10) =>
  ({ name: 'input', type, size }) as File;

test('rejects unsupported types, oversized files, and a fifth image', () => {
  expect(validateImageFile(file('image/gif'), 0)).toContain('JPEG');
  expect(
    validateImageFile(file('image/png', MAX_IMAGE_SOURCE_BYTES + 1), 0),
  ).toContain('20 MiB');
  expect(validateImageFile(file('image/png'), 4)).toContain('four');
  expect(validateImageFile(file('image/webp'), 3)).toBeNull();
});

test('normalizes an accepted image to a bounded JPEG payload', async () => {
  const win = new GlobalWindow();
  const closes: boolean[] = [];
  const draws: number[][] = [];
  const originalCreateImageBitmap = globalThis.createImageBitmap;
  Object.assign(globalThis, {
    window: win,
    document: win.document,
    createImageBitmap: async () => ({
      width: 4000,
      height: 2000,
      close: () => closes.push(true),
    }),
  });
  // happy-dom canvases cannot rasterize — stub the 2d surface so the
  // geometry math and the data-URL split are what the test exercises.
  const originalGetContext = win.HTMLCanvasElement.prototype.getContext;
  const originalToDataURL = win.HTMLCanvasElement.prototype.toDataURL;
  win.HTMLCanvasElement.prototype.getContext = function (
    this: HTMLCanvasElement,
  ) {
    return {
      drawImage: (
        _source: unknown,
        _sx: number,
        _sy: number,
        w: number,
        h: number,
      ) => draws.push([this.width, this.height, w, h]),
    } as unknown as CanvasRenderingContext2D;
  } as typeof win.HTMLCanvasElement.prototype.getContext;
  win.HTMLCanvasElement.prototype.toDataURL = () =>
    'data:image/jpeg;base64,PREVIEW';
  try {
    const image = await normalizeImageFile(
      new File([new Uint8Array([1])], 'diagram.png', { type: 'image/png' }),
      0,
    );

    expect(image.name).toBe('diagram.png');
    expect(image.previewUrl).toBe('data:image/jpeg;base64,PREVIEW');
    expect(image.jpegBase64).toBe('PREVIEW');
    // 4000×2000 scales to the 2048 edge cap, not a straight passthrough.
    expect(draws).toEqual([[2048, 1024, 2048, 1024]]);
    expect(closes).toEqual([true]);
  } finally {
    win.HTMLCanvasElement.prototype.getContext = originalGetContext;
    win.HTMLCanvasElement.prototype.toDataURL = originalToDataURL;
    if (originalCreateImageBitmap === undefined) {
      delete (globalThis as Record<string, unknown>).createImageBitmap;
    } else {
      globalThis.createImageBitmap = originalCreateImageBitmap;
    }
    await win.happyDOM.close();
  }
});

test('normalization throws the validation error unchanged', async () => {
  await expect(
    normalizeImageFile(file('image/gif') as File, 0),
  ).rejects.toThrow('JPEG');
});
