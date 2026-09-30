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
const SITE_URL = 'https://getmarvis.com';
const LOGO_URL = `${SITE_URL}/logo.png`;

export const validateSignup = (input: unknown): { email: string } | null => {
  if (typeof input !== 'object' || input === null) return null;
  const { email } = input as Record<string, unknown>;
  if (typeof email !== 'string') return null;
  const trimmedEmail = email.trim().toLowerCase();
  if (trimmedEmail.length > 254 || !EMAIL_RE.test(trimmedEmail)) return null;
  return { email: trimmedEmail };
};

// First XFF hop is the client; x-real-ip is the fallback. isIP() guards the
// inet column — a garbage header must not 500 the insert.
export const clientIp = (headers: Headers): string | null => {
  const xff = headers.get('x-forwarded-for')?.split(',')[0]?.trim();
  const candidate = xff || headers.get('x-real-ip')?.trim() || null;
  return candidate && isIP(candidate) ? candidate : null;
};

/* PNG logo, not the SVG: Gmail/Outlook/Yahoo don't render SVG <img> at all.
 * public/logo.png is a 396×112 (2× viewBox) render of logo.svg with alpha. */
const WELCOME_TEXT = `Hi there,

Thanks for joining the Marvis waitlist — we'll email you as soon as Marvis is ready to download on macOS, Windows, and Linux.

Marvis is a private AI that floats above your desktop: it sees your screen only with permission, answers in an overlay, and never uploads your keys or screen data.

The Marvis AI Team
${SITE_URL}`;

const WELCOME_HTML = `<!doctype html>
<html lang="en" xmlns="http://www.w3.org/1999/xhtml">
  <head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <meta name="x-apple-disable-message-reformatting" />
    <title>You&#39;re on the Marvis waitlist</title>
  </head>
  <body style="margin:0;padding:0;background-color:#f6f5f4;">
    <div style="display:none;max-height:0;overflow:hidden;mso-hide:all;">Marvis is coming to macOS, Windows, and Linux &mdash; we&rsquo;ll email you when it&rsquo;s ready to download.</div>
    <table role="presentation" width="100%" cellpadding="0" cellspacing="0" border="0" bgcolor="#f6f5f4" style="background-color:#f6f5f4;">
      <tr>
        <td align="center" style="padding:48px 16px;">
          <table role="presentation" width="560" cellpadding="0" cellspacing="0" border="0" style="width:100%;max-width:560px;background-color:#ffffff;border:1px solid #e7e5e3;border-radius:12px;">
            <tr>
              <td style="padding:40px 40px 0;">
                <a href="${SITE_URL}" style="text-decoration:none;"><img src="${LOGO_URL}" width="140" height="40" alt="Marvis" style="display:block;border:0;width:140px;height:auto;" /></a>
              </td>
            </tr>
            <tr>
              <td style="padding:32px 40px 40px;font-family:'Inter',-apple-system,BlinkMacSystemFont,'Segoe UI',Helvetica,Arial,sans-serif;">
                <p style="margin:0 0 14px;font-size:12px;font-weight:600;letter-spacing:0.08em;text-transform:uppercase;color:#507184;">Waitlist confirmed</p>
                <h1 style="margin:0 0 16px;font-size:28px;line-height:1.2;letter-spacing:-0.02em;font-weight:700;color:#31302e;">You&rsquo;re in.</h1>
                <p style="margin:0 0 16px;font-size:16px;line-height:1.6;color:#615d59;">Thanks for joining the Marvis waitlist. Marvis is a private AI that floats above your desktop &mdash; it sees your screen only with permission, answers in an overlay, and never uploads your keys or screen data.</p>
                <p style="margin:0 0 16px;font-size:16px;line-height:1.6;color:#615d59;">We&rsquo;ll email you the moment it&rsquo;s ready to download, with builds for all three desktop platforms:</p>
                <table role="presentation" cellpadding="0" cellspacing="0" border="0" style="margin:0 0 28px;">
                  <tr>
                    <td style="padding-right:8px;"><span style="display:inline-block;padding:6px 14px;border:1px solid #e0dcd8;border-radius:9999px;font-size:13px;font-weight:500;color:#31302e;">macOS</span></td>
                    <td style="padding-right:8px;"><span style="display:inline-block;padding:6px 14px;border:1px solid #e0dcd8;border-radius:9999px;font-size:13px;font-weight:500;color:#31302e;">Windows</span></td>
                    <td><span style="display:inline-block;padding:6px 14px;border:1px solid #e0dcd8;border-radius:9999px;font-size:13px;font-weight:500;color:#31302e;">Linux</span></td>
                  </tr>
                </table>
                <table role="presentation" cellpadding="0" cellspacing="0" border="0" style="margin:0 0 28px;">
                  <tr>
                    <td bgcolor="#78a1bb" style="border-radius:4px;mso-padding-alt:11px 22px;">
                      <a href="${SITE_URL}" style="display:inline-block;padding:11px 22px;font-size:15px;font-weight:600;color:#24231f;text-decoration:none;border-radius:4px;">Visit getmarvis.com</a>
                    </td>
                  </tr>
                </table>
                <p style="margin:0;font-size:16px;line-height:1.6;color:#615d59;">The Marvis AI Team</p>
              </td>
            </tr>
          </table>
          <table role="presentation" width="560" cellpadding="0" cellspacing="0" border="0" style="width:100%;max-width:560px;">
            <tr>
              <td align="center" style="padding:24px 40px 0;font-family:'Inter',-apple-system,BlinkMacSystemFont,'Segoe UI',Helvetica,Arial,sans-serif;font-size:12px;line-height:1.6;color:#a39e98;">
                You&rsquo;re receiving this email because you joined the Marvis waitlist.<br />
                <a href="${SITE_URL}" style="color:#a39e98;text-decoration:underline;">getmarvis.com</a>
              </td>
            </tr>
          </table>
        </td>
      </tr>
    </table>
  </body>
</html>`;

const sendWelcome = async (email: string): Promise<string | null> => {
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
    text: WELCOME_TEXT,
    html: WELCOME_HTML,
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
      'INSERT INTO waitlist (email, ip_address, user_agent) VALUES ($1, $2, $3) ON CONFLICT (email) DO NOTHING RETURNING id',
      [parsed.email, ip, userAgent],
    );
    // rows.length === 0 means a duplicate — success without re-sending mail.
    if (rows.length > 0) {
      const emailId = await sendWelcome(parsed.email).catch((err) => {
        console.error('welcome email failed', err);
        return null;
      });
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
