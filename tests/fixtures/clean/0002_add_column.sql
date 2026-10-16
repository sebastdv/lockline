ALTER TABLE users ADD COLUMN created_at timestamptz DEFAULT now();
