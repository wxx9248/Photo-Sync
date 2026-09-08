-- One device's staging manifest (SPEC.md section 7.6) and its commit write-log (section 7.3),
-- as separate tables in one file on the vault filesystem.

CREATE TABLE staging_entry (
    file          INTEGER PRIMARY KEY,
    path          TEXT    NOT NULL,
    size          INTEGER NOT NULL,
    mtime         INTEGER NOT NULL,

    -- Bytes a completed sync has made durable. Recovery truncates a partial to this.
    durable_bytes INTEGER NOT NULL,

    -- Set once the streamed digest matched what the phone stated.
    digest        BLOB,

    -- Wall-clock readings a vault name may be built from, in the order of section 7.2.
    name_sources  TEXT    NOT NULL
) WITHOUT ROWID;

-- The write-log. Section 7.3 says to write the map, flush, and seal it with an end marker;
-- one transaction does all three, so a half-written log physically cannot exist. The presence
-- of rows is the seal.
CREATE TABLE commit_plan (
    file       INTEGER PRIMARY KEY,

    -- The order the plan is executed and replayed in.
    ordinal    INTEGER NOT NULL,

    -- 'import' moves the staged file into the vault; 'duplicate' deletes it.
    action     TEXT    NOT NULL,
    vault_name TEXT    NOT NULL,

    -- Written only after the directory syncs for its group have returned.
    done       INTEGER NOT NULL DEFAULT 0
) WITHOUT ROWID;
