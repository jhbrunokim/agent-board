-- Initial committed schema, preserved as an upgrade fixture.
CREATE TABLE IF NOT EXISTS schema_version (version BIGINT PRIMARY KEY);
CREATE TABLE IF NOT EXISTS projects (id TEXT PRIMARY KEY, name TEXT NOT NULL, head BIGINT NOT NULL DEFAULT 0);
CREATE TABLE IF NOT EXISTS tokens (hash TEXT PRIMARY KEY, project_id TEXT NOT NULL REFERENCES projects(id), principal TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS threads (id TEXT PRIMARY KEY, project_id TEXT NOT NULL REFERENCES projects(id), title TEXT NOT NULL, UNIQUE(project_id,id));
CREATE TABLE IF NOT EXISTS messages (id TEXT PRIMARY KEY, project_id TEXT NOT NULL, thread_id TEXT NOT NULL, sender TEXT NOT NULL, seq BIGINT NOT NULL, payload TEXT NOT NULL, FOREIGN KEY(project_id,thread_id) REFERENCES threads(project_id,id));
CREATE TABLE IF NOT EXISTS events (project_id TEXT NOT NULL REFERENCES projects(id), seq BIGINT NOT NULL, body TEXT NOT NULL, PRIMARY KEY(project_id,seq));
CREATE TABLE IF NOT EXISTS dedup (project_id TEXT NOT NULL, principal TEXT NOT NULL, operation TEXT NOT NULL, key TEXT NOT NULL, hash TEXT NOT NULL, response TEXT NOT NULL, PRIMARY KEY(project_id,principal,operation,key));
CREATE INDEX IF NOT EXISTS messages_thread ON messages(project_id,thread_id);
INSERT INTO schema_version(version) VALUES (1) ON CONFLICT DO NOTHING;
