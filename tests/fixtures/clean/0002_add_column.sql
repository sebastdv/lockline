SET lock_timeout = '5s';
ALTER TABLE users ADD COLUMN created_at timestamptz DEFAULT now();
