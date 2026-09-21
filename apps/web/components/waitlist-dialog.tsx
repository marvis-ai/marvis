'use client';

import {
  Button,
  Dialog,
  DialogClose,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
  DialogTrigger,
  Input,
  Label,
  XIcon,
} from '@marvis/ui';
import { useId, useState } from 'react';
import type { SubmitEvent } from 'react';

type Status = 'idle' | 'submitting' | 'success' | 'error';

export const WaitlistDialog = ({
  triggerClassName,
  triggerLabel,
  triggerDataOdId,
}: {
  triggerClassName: string;
  triggerLabel: string;
  triggerDataOdId: string;
}) => {
  const uid = useId();
  const [status, setStatus] = useState<Status>('idle');
  const [error, setError] = useState('');

  const onSubmit = async (e: SubmitEvent<HTMLFormElement>) => {
    e.preventDefault();
    const data = new FormData(e.currentTarget);
    setStatus('submitting');
    setError('');
    try {
      const res = await fetch('/api/waitlist', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          email: data.get('email'),
        }),
      });
      setStatus(res.ok ? 'success' : 'error');
      if (res.status === 400)
        setError('Please check your email address, then try again.');
      else if (!res.ok) setError('Something went wrong — please try again.');
    } catch {
      setStatus('error');
      setError('Network error — please try again.');
    }
  };

  return (
    <Dialog onOpenChange={() => setStatus('idle')}>
      <DialogTrigger
        render={
          <button
            type='button'
            className={triggerClassName}
            data-od-id={triggerDataOdId}
          />
        }>
        {triggerLabel}
      </DialogTrigger>
      <DialogContent showCloseButton={false}>
        <DialogHeader>
          <p className='eyebrow'>Waitlist</p>
          <DialogTitle>Get Marvis for macOS</DialogTitle>
          <DialogDescription>
            Join the early-access list — we&rsquo;ll email you a download link
            when the macOS build is ready.
          </DialogDescription>
        </DialogHeader>

        <DialogClose
          className='waitlist-close'
          aria-label='Close'>
          <XIcon aria-hidden='true' />
        </DialogClose>

        {status === 'success' ? (
          <div className='waitlist-done'>
            <p className='pill pill-green'>You&rsquo;re on the list</p>
            <p className='waitlist-note'>
              No account, no cloud sync — your keys and screen data stay on your
              machine.
            </p>
          </div>
        ) : (
          <form
            onSubmit={onSubmit}
            className='grid gap-4'>
            <div className='grid gap-2'>
              <Label htmlFor={`${uid}-email`}>Email</Label>
              <Input
                id={`${uid}-email`}
                name='email'
                type='email'
                required
                maxLength={254}
                autoComplete='email'
                placeholder='ada@example.com'
              />
            </div>
            {/* Honeypot — hidden from humans, filled by bots */}
            <input
              name='company'
              type='text'
              tabIndex={-1}
              autoComplete='off'
              aria-hidden='true'
              style={{
                position: 'absolute',
                left: '-100vw',
                width: 0,
                height: 0,
                opacity: 0,
              }}
            />
            {status === 'error' && <p className='waitlist-error'>{error}</p>}
            <Button
              type='submit'
              className='btn btn-primary'
              disabled={status === 'submitting'}>
              {status === 'submitting' ? 'Joining…' : 'Join the waitlist'}
            </Button>
          </form>
        )}
      </DialogContent>
    </Dialog>
  );
};
