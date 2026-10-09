/// <reference types="bun-types" />
import { expect, test } from 'bun:test';
import { GlobalWindow } from 'happy-dom';
import { act } from 'react';
import { createRoot } from 'react-dom/client';
import { AskAttachments } from '@/components/bar/AskAttachments';

const win = () => {
  const w = new GlobalWindow();
  Object.assign(globalThis, {
    window: w,
    document: w.document,
    IS_REACT_ACT_ENVIRONMENT: true,
  });
  return w;
};

test('renders pending previews and removes one attachment', async () => {
  const w = win();
  const removed: number[] = [];
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = createRoot(host);
  try {
    await act(async () =>
      root.render(
        <AskAttachments
          images={[
            {
              id: 'p1',
              name: 'one.png',
              jpegBase64: 'ONE',
              previewUrl: 'data:image/jpeg;base64,ONE',
            },
            {
              id: 'p2',
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
    (
      host.querySelector('[aria-label="Remove two.webp"]') as HTMLButtonElement
    ).click();
    expect(removed).toEqual([1]);
  } finally {
    await act(async () => root.unmount());
    host.remove();
    await w.happyDOM.close();
  }
});

test('shows the validation error and nothing at all when empty', async () => {
  const w = win();
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = createRoot(host);
  try {
    await act(async () =>
      root.render(
        <AskAttachments
          images={[]}
          error='At most four images can be attached to one message'
          onRemove={() => {}}
        />,
      ),
    );
    expect(host.textContent).toContain('four images');

    await act(async () =>
      root.render(
        <AskAttachments
          images={[]}
          error={null}
          onRemove={() => {}}
        />,
      ),
    );
    expect(host.childElementCount).toBe(0);
  } finally {
    await act(async () => root.unmount());
    host.remove();
    await w.happyDOM.close();
  }
});
