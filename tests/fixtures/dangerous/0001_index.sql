-- Looks harmless, locks writes on a big table.
CREATE INDEX idx_users_created_at ON users (created_at);
