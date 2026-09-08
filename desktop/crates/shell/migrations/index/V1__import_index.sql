-- The import index of SPEC.md section 7.5. It lives outside the vault, so curating or
-- relocating the vault never touches it.

CREATE TABLE content (
    digest     BLOB PRIMARY KEY,
    vault_name TEXT NOT NULL
) WITHOUT ROWID;

-- Keyed by device and path, so re-importing a changed file replaces the row it supersedes
-- rather than adding a second one. That mirrors the phone, where the edit replaced the
-- original.
CREATE TABLE device_file (
    device_id    TEXT    NOT NULL,
    device_path  TEXT    NOT NULL,
    size         INTEGER NOT NULL,
    mtime        INTEGER NOT NULL,
    digest       BLOB    NOT NULL,
    vault_name   TEXT    NOT NULL,
    committed_at INTEGER NOT NULL,
    PRIMARY KEY (device_id, device_path)
) WITHOUT ROWID;
