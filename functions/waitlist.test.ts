import { describe, expect, test } from 'bun:test';
import app, { validateSignup } from './waitlist';

describe('validateSignup', () => {
  test('accepts a valid signup and normalizes it', () => {
    expect(
      validateSignup({ name: '  Ada  ', email: ' Ada@Example.COM ' }),
    ).toEqual({
      name: 'Ada',
      email: 'ada@example.com',
    });
  });

  test.each([
    ['null', null],
    ['non-object', 'x'],
    ['missing email', { name: 'Ada' }],
    ['non-string email', { name: 'Ada', email: 1 }],
    ['bad email', { name: 'Ada', email: 'nope' }],
    ['empty name', { name: '   ', email: 'a@b.co' }],
    ['too-long name', { name: 'x'.repeat(121), email: 'a@b.co' }],
    ['too-long email', { name: 'Ada', email: `${'a'.repeat(250)}@b.co` }],
  ])('rejects %s', (_label: string, input: unknown) => {
    expect(validateSignup(input)).toBeNull();
  });
});

describe('POST / honeypot', () => {
  // The `company` trap must catch any non-empty value — a bot posting a
  // number must get the same fake success as one posting a string, and the
  // request is discarded before touching the database.
  test.each([
    ['string', 'spam'],
    ['number', 123],
  ])(
    'fakes success for a %s company value',
    async (_label: string, company: unknown) => {
      const res = await app.request('/', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ name: 'Bot', email: 'bot@x.co', company }),
      });
      expect(res.status).toBe(200);
      expect(await res.json()).toEqual({ ok: true });
    },
  );
});
