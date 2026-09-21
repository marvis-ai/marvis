CREATE EXTENSION IF NOT EXISTS citext;

CREATE TABLE IF NOT EXISTS waitlist (
  id         uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  name       text NOT NULL,
  email      citext NOT NULL UNIQUE,
  created_at timestamptz NOT NULL DEFAULT now()
);
