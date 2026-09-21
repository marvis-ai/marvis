'use client';

import {
  Button,
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
  DialogTrigger,
  Input,
  Label,
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
          name: data.get('name'),
          email: data.get('email'),
          company: data.get('company'),
        }),
      });
      setStatus(res.ok ? 'success' : 'error');
      if (!res.ok) setError('Something went wrong — please try again.');
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
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Get Marvis for macOS</DialogTitle>
          <DialogDescription>
            Join the waitlist — we&rsquo;ll email you when the build is ready.
          </DialogDescription>
        </DialogHeader>
        {status === 'success' ? (
          <p className='text-sm text-muted-foreground'>
            You&rsquo;re on the list — check your inbox for a confirmation.
          </p>
        ) : (
          <form
            onSubmit={onSubmit}
            className='grid gap-4'>
            <div className='grid gap-2'>
              <Label htmlFor={`${uid}-name`}>Name</Label>
              <Input
                id={`${uid}-name`}
                name='name'
                required
                maxLength={120}
                autoComplete='name'
                placeholder='Ada Lovelace'
              />
            </div>
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
            {status === 'error' && (
              <p className='text-sm text-destructive'>{error}</p>
            )}
            <Button
              type='submit'
              disabled={status === 'submitting'}>
              {status === 'submitting' ? 'Joining…' : 'Join the waitlist'}
            </Button>
          </form>
        )}
      </DialogContent>
    </Dialog>
  );
};
