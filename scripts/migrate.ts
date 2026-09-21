import { readFileSync, readdirSync } from 'node:fs';
import { Pool } from 'pg';

const pool = new Pool({ connectionString: process.env.DATABASE_URL });
const dir = new URL('../db/', import.meta.url);
const files = readdirSync(dir)
  .filter((f) => f.endsWith('.sql'))
  .sort();

for (const file of files) {
  await pool.query(readFileSync(new URL(file, dir), 'utf8'));
  console.log(`applied ${file}`);
}
const { rows } = await pool.query('SELECT count(*)::int AS n FROM waitlist');
console.log(`waitlist table ready — ${rows[0].n} row(s)`);
await pool.end();
