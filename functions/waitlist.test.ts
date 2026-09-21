import { describe, expect, test } from 'bun:test';
import app, { clientIp, validateSignup } from './waitlist';

describe('validateSignup', () => {
  test('accepts a valid signup and normalizes it', () => {
    expect(
      validateSignup({ name: '  Ada  ', email: ' Ada@Example.COM ' }),
    ).toEqual({
      name: 'Ada',
      email: 'ada@example.com',
    });
  });

  // Name is optional — the form collects email only. A missing, blank, or
  // non-string name normalizes to '' (the DB column is NOT NULL, so '' keeps
  // the insert honest; the welcome email falls back to "Hi there").
  test.each([
    ['missing name', { email: 'a@b.co' }],
    ['blank name', { name: '   ', email: 'a@b.co' }],
    ['non-string name', { name: 1, email: 'a@b.co' }],
  ])('accepts an email-only signup (%s)', (_label, input) => {
    expect(validateSignup(input)).toEqual({
      name: '',
      email: 'a@b.co',
    });
  });

  test.each([
    ['null', null],
    ['non-object', 'x'],
    ['missing email', { name: 'Ada' }],
    ['non-string email', { name: 'Ada', email: 1 }],
    ['bad email', { name: 'Ada', email: 'nope' }],
    ['too-long name', { name: 'x'.repeat(121), email: 'a@b.co' }],
    ['too-long email', { name: 'Ada', email: `${'a'.repeat(250)}@b.co` }],
  ])('rejects %s', (_label: string, input: unknown) => {
    expect(validateSignup(input)).toBeNull();
  });
});

describe('clientIp', () => {
  // The inet column rejects garbage — every branch must yield a valid IP
  // or null, never raw header text.
  test('takes the first x-forwarded-for hop', () => {
    const h = new Headers({ 'x-forwarded-for': '203.0.113.7, 10.0.0.1' });
    expect(clientIp(h)).toBe('203.0.113.7');
  });

  test('falls back to x-real-ip', () => {
    expect(clientIp(new Headers({ 'x-real-ip': '2001:db8::1' }))).toBe(
      '2001:db8::1',
    );
  });

  test.each([
    ['garbage xff', { 'x-forwarded-for': 'not-an-ip' }],
    ['empty xff', { 'x-forwarded-for': '' }],
    ['no headers', {}],
  ])('returns null for %s', (_label: string, init: HeadersInit) => {
    expect(clientIp(new Headers(init))).toBeNull();
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
