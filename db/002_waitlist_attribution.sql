ALTER TABLE waitlist
  ADD COLUMN IF NOT EXISTS ip_address       inet,
  ADD COLUMN IF NOT EXISTS user_agent       text,
  ADD COLUMN IF NOT EXISTS email_sent_at    timestamptz,
  ADD COLUMN IF NOT EXISTS resend_email_id  text;
