CREATE TABLE users (
    id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    email text NOT NULL
);

-- Safe: the table was created in this same migration, so it has no rows or traffic yet.
CREATE INDEX idx_users_email ON users (email);
