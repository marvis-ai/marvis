import { describe, expect, test } from 'bun:test';
import { validateSignup } from './waitlist';

describe('validateSignup', () => {
  test('accepts a valid signup and normalizes it', () => {
    expect(validateSignup({ name: '  Ada  ', email: ' Ada@Example.COM ' })).toEqual({
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
  ])('rejects %s', (_label, input) => {
    expect(validateSignup(input)).toBeNull();
  });
});
