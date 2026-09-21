import { attachDatabasePool } from '@neon/functions';
import { Hono } from 'hono';
import { Pool } from 'pg';
import { Resend } from 'resend';

/* Pool lives at module scope — the isolate persists across requests.
 * attachDatabasePool swallows idle-client drops so they don't crash it. */
const pool = new Pool({ connectionString: process.env.DATABASE_URL, max: 5 });
attachDatabasePool(pool);

const EMAIL_RE = /^[^\s@]+@[^\s@]+\.[^\s@]+$/;
const DEFAULT_FROM = 'Marvis AI <notification@updates.getmarvis.com>';

export const validateSignup = (input: unknown): { name: string; email: string } | null => {
  if (typeof input !== 'object' || input === null) return null;
  const { name, email } = input as Record<string, unknown>;
  if (typeof name !== 'string' || typeof email !== 'string') return null;
  const trimmedName = name.trim();
  const trimmedEmail = email.trim().toLowerCase();
  if (trimmedName.length < 1 || trimmedName.length > 120) return null;
  if (trimmedEmail.length > 254 || !EMAIL_RE.test(trimmedEmail)) return null;
  return { name: trimmedName, email: trimmedEmail };
};

const escapeHtml = (s: string) =>
  s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;');

const sendWelcome = async (name: string, email: string) => {
  const key = process.env.RESEND_API_KEY;
  if (!key) {
    console.warn('RESEND_API_KEY unset — skipping waitlist email');
    return;
  }
  const resend = new Resend(key);
  const { error } = await resend.emails.send({
    from: process.env.FROM_EMAIL ?? DEFAULT_FROM,
    to: email,
    subject: "You're on the Marvis waitlist",
    text: `Hi ${name},\n\nThanks for joining the Marvis waitlist — we'll email you as soon as the macOS build is ready to download.\n\n— The Marvis team`,
    html: `<p>Hi ${escapeHtml(name)},</p><p>Thanks for joining the Marvis waitlist — we'll email you as soon as the macOS build is ready to download.</p><p>— The Marvis team</p>`,
  });
  if (error) console.error('Resend send failed', error);
};

const app = new Hono();

app.get('/', (c) => c.json({ ok: true }));

app.post('/', async (c) => {
  const body = (await c.req.json().catch(() => null)) as Record<string, unknown> | null;
  // Honeypot: the hidden `company` field is invisible to humans; a filled
  // value means a bot — fake success and discard.
  if (typeof body?.company === 'string' && body.company.trim() !== '') {
    return c.json({ ok: true });
  }
  const parsed = validateSignup(body);
  if (!parsed) return c.json({ ok: false, error: 'invalid' }, 400);
  try {
    const { rows } = await pool.query(
      'INSERT INTO waitlist (name, email) VALUES ($1, $2) ON CONFLICT (email) DO NOTHING RETURNING id',
      [parsed.name, parsed.email],
    );
    // rows.length === 0 means a duplicate — success without re-sending mail.
    if (rows.length > 0) {
      await sendWelcome(parsed.name, parsed.email).catch((err) =>
        console.error('welcome email failed', err),
      );
    }
    return c.json({ ok: true });
  } catch (err) {
    console.error('waitlist insert failed', err);
    return c.json({ ok: false, error: 'server' }, 500);
  }
});

export default app;
