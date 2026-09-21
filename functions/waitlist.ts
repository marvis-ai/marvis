import { attachDatabasePool } from '@neon/functions';
import { Hono } from 'hono';
import { isIP } from 'node:net';
import { Pool } from 'pg';
import { Resend } from 'resend';

/* Pool lives at module scope — the isolate persists across requests.
 * attachDatabasePool swallows idle-client drops so they don't crash it. */
const pool = new Pool({ connectionString: process.env.DATABASE_URL, max: 5 });
attachDatabasePool(pool);

const EMAIL_RE = /^[^\s@]+@[^\s@]+\.[^\s@]+$/;
const DEFAULT_FROM = 'Marvis AI <notification@updates.getmarvis.com>';

export const validateSignup = (
  input: unknown,
): { name: string; email: string } | null => {
  if (typeof input !== 'object' || input === null) return null;
  const { name, email } = input as Record<string, unknown>;
  if (typeof name !== 'string' || typeof email !== 'string') return null;
  const trimmedName = name.trim();
  const trimmedEmail = email.trim().toLowerCase();
  if (trimmedName.length < 1 || trimmedName.length > 120) return null;
  if (trimmedEmail.length > 254 || !EMAIL_RE.test(trimmedEmail)) return null;
  return { name: trimmedName, email: trimmedEmail };
};

// First XFF hop is the client; x-real-ip is the fallback. isIP() guards the
// inet column — a garbage header must not 500 the insert.
export const clientIp = (headers: Headers): string | null => {
  const xff = headers.get('x-forwarded-for')?.split(',')[0]?.trim();
  const candidate = xff || headers.get('x-real-ip')?.trim() || null;
  return candidate && isIP(candidate) ? candidate : null;
};

const escapeHtml = (s: string) =>
  s
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;');

const sendWelcome = async (
  name: string,
  email: string,
): Promise<string | null> => {
  const key = process.env.RESEND_API_KEY;
  if (!key) {
    console.warn('RESEND_API_KEY unset — skipping waitlist email');
    return null;
  }
  const resend = new Resend(key);
  const { data, error } = await resend.emails.send({
    from: process.env.FROM_EMAIL ?? DEFAULT_FROM,
    to: email,
    subject: "You're on the Marvis waitlist",
    text: `Hi ${name},\n\nThanks for joining the Marvis waitlist — we'll email you as soon as the macOS build is ready to download.\n\n— The Marvis team`,
    html: `<p>Hi ${escapeHtml(name)},</p><p>Thanks for joining the Marvis waitlist — we'll email you as soon as the macOS build is ready to download.</p><p>— The Marvis team</p>`,
  });
  if (error) {
    console.error('Resend send failed', error);
    return null;
  }
  return data?.id ?? null;
};

const app = new Hono();

app.get('/', (c) => c.json({ ok: true }));

app.post('/', async (c) => {
  const body = (await c.req.json().catch(() => null)) as Record<
    string,
    unknown
  > | null;
  // Honeypot: the hidden `company` field is invisible to humans; any filled
  // value (even a non-string one a bot posted) means a bot — fake success
  // and discard.
  if (body?.company != null && String(body.company).trim() !== '') {
    return c.json({ ok: true });
  }
  const parsed = validateSignup(body);
  if (!parsed) return c.json({ ok: false, error: 'invalid' }, 400);
  const ip = clientIp(c.req.raw.headers);
  const userAgent = c.req.header('user-agent')?.slice(0, 512) ?? null;
  try {
    const { rows } = await pool.query(
      'INSERT INTO waitlist (name, email, ip_address, user_agent) VALUES ($1, $2, $3, $4) ON CONFLICT (email) DO NOTHING RETURNING id',
      [parsed.name, parsed.email, ip, userAgent],
    );
    // rows.length === 0 means a duplicate — success without re-sending mail.
    if (rows.length > 0) {
      const emailId = await sendWelcome(parsed.name, parsed.email).catch(
        (err) => {
          console.error('welcome email failed', err);
          return null;
        },
      );
      // NULL email_sent_at marks signups whose welcome never went out —
      // retryable later. Stamping must not fail the signup.
      if (emailId) {
        await pool
          .query(
            'UPDATE waitlist SET email_sent_at = now(), resend_email_id = $2 WHERE id = $1',
            [rows[0].id, emailId],
          )
          .catch((err) => console.error('email_sent_at stamp failed', err));
      }
    }
    return c.json({ ok: true });
  } catch (err) {
    console.error('waitlist insert failed', err);
    return c.json({ ok: false, error: 'server' }, 500);
  }
});

export default app;
