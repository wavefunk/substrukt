CREATE TABLE IF NOT EXISTS webhook_state (
    environment TEXT PRIMARY KEY,
    last_fired_at TEXT
);

INSERT OR IGNORE INTO webhook_state (environment, last_fired_at) VALUES ('staging', NULL);
INSERT OR IGNORE INTO webhook_state (environment, last_fired_at) VALUES ('production', NULL);
