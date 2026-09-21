import { readFileSync } from 'node:fs';
import { Pool } from 'pg';

const pool = new Pool({ connectionString: process.env.DATABASE_URL });
const ddl = readFileSync(new URL('../db/001_waitlist.sql', import.meta.url), 'utf8');

await pool.query(ddl);
const { rows } = await pool.query('SELECT count(*)::int AS n FROM waitlist');
console.log(`waitlist table ready — ${rows[0].n} row(s)`);
await pool.end();
