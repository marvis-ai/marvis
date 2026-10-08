/// <reference types="bun-types" />
import { expect, test } from 'bun:test';
import { GlobalWindow } from 'happy-dom';
import { act } from 'react';
import { createRoot } from 'react-dom/client';
import { SpeakerNameEditor } from './SpeakerNameEditor';
import { SpeakerFilter } from './SpeakerFilter';
import { TranscriptBlocks } from './TranscriptBlocks';
import type { TurnBlock } from './model';

/** Drive a controlled React input: the native setter mutates `.value`,
 *  then an `input` event lets React's synthetic `onChange` see it. */
const typeInto = async (
  win: GlobalWindow,
  input: HTMLInputElement,
  value: string,
) => {
  Object.getOwnPropertyDescriptor(
    win.HTMLInputElement.prototype,
    'value',
  )!.set!.call(input, value);
  await act(async () =>
    input.dispatchEvent(
      new win.Event('input', { bubbles: true }) as unknown as Event,
    ),
  );
};

const press = async (
  win: GlobalWindow,
  input: HTMLInputElement,
  key: string,
) => {
  await act(async () =>
    input.dispatchEvent(
      new win.KeyboardEvent('keydown', {
        key,
        bubbles: true,
        cancelable: true,
      }) as unknown as Event,
    ),
  );
};

test('edits on Enter, trims, and caps the committed label', async () => {
  const win = new GlobalWindow();
  Object.assign(globalThis, {
    window: win,
    document: win.document,
    IS_REACT_ACT_ENVIRONMENT: true,
  });
  const committed: string[] = [];
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = createRoot(host);
  try {
    await act(async () =>
      root.render(
        <SpeakerNameEditor
          label='Speaker 1'
          editable
          onCommit={(value) => committed.push(value)}
        />,
      ),
    );
    await act(async () =>
      (host.querySelector('button') as HTMLButtonElement).click(),
    );
    const input = host.querySelector('input') as HTMLInputElement;
    expect(input).not.toBeNull();
    await typeInto(win, input, `  ${'x'.repeat(50)}  `);
    await press(win, input, 'Enter');

    expect(committed).toEqual(['x'.repeat(40)]);
  } finally {
    await act(async () => root.unmount());
    host.remove();
    await win.happyDOM.close();
  }
});

test('Escape cancels without committing', async () => {
  const win = new GlobalWindow();
  Object.assign(globalThis, {
    window: win,
    document: win.document,
    IS_REACT_ACT_ENVIRONMENT: true,
  });
  const committed: string[] = [];
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = createRoot(host);
  try {
    await act(async () =>
      root.render(
        <SpeakerNameEditor
          label='Speaker 1'
          editable
          onCommit={(value) => committed.push(value)}
        />,
      ),
    );
    await act(async () =>
      (host.querySelector('button') as HTMLButtonElement).click(),
    );
    const input = host.querySelector('input') as HTMLInputElement;
    await typeInto(win, input, 'Alice');
    await press(win, input, 'Escape');

    expect(committed).toEqual([]);
    expect(host.querySelector('input')).toBeNull();
    expect(host.textContent).toContain('Speaker 1');
  } finally {
    await act(async () => root.unmount());
    host.remove();
    await win.happyDOM.close();
  }
});

test('an empty commit reaches onCommit so the parent can drop the override', async () => {
  const win = new GlobalWindow();
  Object.assign(globalThis, {
    window: win,
    document: win.document,
    IS_REACT_ACT_ENVIRONMENT: true,
  });
  const committed: string[] = [];
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = createRoot(host);
  try {
    await act(async () =>
      root.render(
        <SpeakerNameEditor
          label='Speaker 1'
          editable
          onCommit={(value) => committed.push(value)}
        />,
      ),
    );
    await act(async () =>
      (host.querySelector('button') as HTMLButtonElement).click(),
    );
    const input = host.querySelector('input') as HTMLInputElement;
    await typeInto(win, input, '   ');
    await press(win, input, 'Enter');

    expect(committed).toEqual(['']);
  } finally {
    await act(async () => root.unmount());
    host.remove();
    await win.happyDOM.close();
  }
});

const block: TurnBlock = {
  key: 'them:0',
  name: 'Speaker 1',
  canRename: true,
  color: 'text-speaker-1',
  ts: 0,
  audioStartMs: null,
  finals: [
    {
      speaker: 'them',
      speaker_idx: 0,
      audio_start_ms: null,
      text: 'hi',
      ts: 0,
      session_id: 1,
      final: true,
    },
  ],
  interim: null,
};

test('a speaker chip renames through onRename, not onPick', async () => {
  const win = new GlobalWindow();
  Object.assign(globalThis, {
    window: win,
    document: win.document,
    IS_REACT_ACT_ENVIRONMENT: true,
  });
  const renames: [string, string][] = [];
  const picks: (string | null)[] = [];
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = createRoot(host);
  try {
    await act(async () =>
      root.render(
        <SpeakerFilter
          speakers={[
            {
              key: 'them:0',
              name: 'Speaker 1',
              color: 'text-speaker-1',
              canRename: true,
            },
          ]}
          active={null}
          count={1}
          elapsed={null}
          copied={false}
          exported={false}
          onPick={(key) => picks.push(key)}
          onRename={(key, label) => renames.push([key, label])}
          onCopy={() => {}}
          onCopyMarkdown={() => {}}
          onSaveMarkdown={() => {}}
          onSaveAudio={() => {}}
          canSaveAudio={false}
        />,
      ),
    );
    await act(async () =>
      (
        host.querySelector(
          '[aria-label="Rename Speaker 1"]',
        ) as HTMLButtonElement
      ).click(),
    );
    const input = host.querySelector('input') as HTMLInputElement;
    await typeInto(win, input, 'Alice');
    await press(win, input, 'Enter');

    expect(renames).toEqual([['them:0', 'Alice']]);
    expect(picks).toEqual([]);
  } finally {
    await act(async () => root.unmount());
    host.remove();
    await win.happyDOM.close();
  }
});

test('a transcript header renames through onRename', async () => {
  const win = new GlobalWindow();
  Object.assign(globalThis, {
    window: win,
    document: win.document,
    IS_REACT_ACT_ENVIRONMENT: true,
  });
  const renames: [string, string][] = [];
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = createRoot(host);
  try {
    await act(async () =>
      root.render(
        <TranscriptBlocks
          blocks={[block]}
          startedAt={null}
          onRename={(key, label) => renames.push([key, label])}
        />,
      ),
    );
    await act(async () =>
      (
        host.querySelector(
          '[aria-label="Rename Speaker 1"]',
        ) as HTMLButtonElement
      ).click(),
    );
    const input = host.querySelector('input') as HTMLInputElement;
    await typeInto(win, input, 'Alice');
    await press(win, input, 'Enter');

    expect(renames).toEqual([['them:0', 'Alice']]);
  } finally {
    await act(async () => root.unmount());
    host.remove();
    await win.happyDOM.close();
  }
});

test('a non-editable label renders plain text with no affordance', async () => {
  const win = new GlobalWindow();
  Object.assign(globalThis, {
    window: win,
    document: win.document,
    IS_REACT_ACT_ENVIRONMENT: true,
  });
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = createRoot(host);
  try {
    await act(async () =>
      root.render(
        <SpeakerNameEditor
          label='You'
          editable={false}
          onCommit={() => {}}
        />,
      ),
    );
    expect(host.querySelector('button')).toBeNull();
    expect(host.querySelector('input')).toBeNull();
    expect(host.textContent).toBe('You');
  } finally {
    await act(async () => root.unmount());
    host.remove();
    await win.happyDOM.close();
  }
});
