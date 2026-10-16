ALTER TABLE users ADD COLUMN last_login_at timestamptz;

CREATE INDEX idx_users_last_login_at ON users (last_login_at);

ALTER TABLE users ADD COLUMN api_token uuid DEFAULT gen_random_uuid();
