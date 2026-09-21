/**
 * About — the supplied wordmark (never redrawn), one-line pitch, and
 * metadata rows. Version resolves from the bundle via `getVersion()`
 * with the Cargo.toml value as fallback.
 */
import { useEffect, useState } from 'react';
import { getVersion } from '@tauri-apps/api/app';
import { PrefRow, Tag } from './bits';

export const AboutTab = () => {
  const [version, setVersion] = useState('0.1.0');

  useEffect(() => {
    void getVersion()
      .then(setVersion)
      .catch(() => {});
  }, []);

  return (
    <>
      <img
        className='prf-about-mark'
        src='/marvis-logo.svg'
        alt='Marvis'
      />
      <p className='prf-about-sub'>
        Sees your screen. Answers in place. Nothing else.
      </p>

      <div className='prf-rows'>
        <PrefRow label='Version'>
          <Tag>{version}</Tag>
        </PrefRow>
        <PrefRow label='License'>
          <Tag>MIT</Tag>
        </PrefRow>
        <PrefRow
          label='Platforms'
          last>
          <span className='num'>macOS now · windows / linux planned</span>
        </PrefRow>
      </div>
    </>
  );
};
