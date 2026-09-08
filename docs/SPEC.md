# Photo Sync — Technical Specification

## 1. Background & goals

The target users are two family members who accumulate photos and videos on Android phones
until storage runs out, then export them to the home computer over MTP/USB — slow, manual,
and unreliable (one phone has a flaky USB port).

This project replaces that workflow with a two-part suite operating over the home LAN:

* **Mobile app** (Android): pushes the entire camera roll to the desktop automatically.
* **Desktop app** (Linux): receives, verifies, deduplicates, renames by capture time, and
  archives photos into a vault; owns *all* transfer state.

Design priorities, in order:

1. **No data loss, ever** — crash-safe at every step; phone-side deletion must be provably safe.
2. **Near-zero interaction** — 2 taps per session after one-time onboarding.
3. **Robustness on hostile OEM Android builds** — the target phones are aggressive app-killers.
4. **Efficient transfer** — saturate the LAN with concurrent streams; resume after interruption.

Throughout this document, "photo" means any in-scope image **or video**.

## 2. Definitions

| Term | Meaning |
|---|---|
| **Catalog** | The phone's listing of all in-scope files, **frozen in RAM at session start**. Photos taken during a session are picked up next session. |
| **Staging** | Desktop's per-device holding area for verified-but-uncommitted files **and in-progress partial transfers**. |
| **Vault** | The final archive directory on the desktop. |
| **Index** | Desktop's persistent record of everything ever imported (survives commits). Stored in the app's data directory, deliberately outside the vault (§7.5). |
| **Session** | One app-launch-to-completion cycle on the phone. A session survives connection drops (§6, *Reconnection*); it does not survive app death — a relaunch starts a new session. |
| **Device ID** | `ANDROID_ID` — hardware serial is unreadable by normal apps since Android 10. Stable per device + app signing key; resets on factory reset. Staging directories and index rows are keyed by it. The handshake also carries a human-readable device name for UI display. |

## 3. Mobile app

Platform: **Android ≥ 12**, Play-Store-compliant permission model.
Languages: **Simplified Chinese (primary) + English**, i18n from day one.

### 3.1 One-time onboarding

Guided, sequential flow:

1. Media read permission — `READ_MEDIA_IMAGES` / `READ_MEDIA_VIDEO` (Android 13+),
   `READ_EXTERNAL_STORAGE` (Android 12).
2. **Media management access** (`MANAGE_MEDIA`, via `ACTION_REQUEST_MANAGE_MEDIA`) — enables
   silent, dialog-free deletion after in-app confirmation. Skippable; see §8 fallback.
3. Battery-optimization exemption (`REQUEST_IGNORE_BATTERY_OPTIMIZATIONS`).
4. Brand-specific keep-alive steps. Target devices are **Xiaomi/Redmi/POCO (MIUI/HyperOS)**
   and **Oppo/Vivo/OnePlus/realme (ColorOS/OriginOS/OxygenOS)** — among the most aggressive
   task-killers. Onboarding shows per-brand illustrated steps: autostart permission, battery
   saver exemption, lock-in-recents.
5. Pairing with the desktop (§5.2).

### 3.2 Catalog

* Scope (v1): the **`DCIM/Camera` bucket only**, images + videos, enumerated via MediaStore.
* Excluded: `IS_PENDING` items (e.g. a video still being recorded) and trashed items.
* Entry identity: **(relative path, size, mtime)**, where *relative path* is MediaStore
  `RELATIVE_PATH` + `DISPLAY_NAME` — it keys the index, the staging manifest, and deletion
  candidates. No upfront hashing — sessions start instantly even on 50 GB libraries. sha256
  is computed while streaming each file (§6).
* The catalog is enumerated **once**, at session start, and held frozen in RAM for the
  session's whole life (including reconnects, §6). Photos taken *during* a transfer are
  simply picked up next session.

### 3.3 Keep-alive stack

Foreground service (`dataSync` type) + partial wake lock + Wi-Fi lock + persistent
notification with live progress. The transfer auto-reconnects and resumes across network
hiccups by re-entering the normal session flow with the frozen catalog (§6, *Reconnection*)
— the user is never re-prompted. A session survives screen-off and backgrounding.

### 3.4 Interaction budget: 2 taps per session

* **Tap 1 — Start**, on a summary screen: "1,180 new photos, 6.4 GB — 2,034 already safe."
* **Tap 2 — Confirm deletion**, at the end (§8).
* **Nothing to send** → the phone auto-sends the finish signal with no tap, so any leftover
  staged files still get committed (§7.3). Then:
  * nothing to delete either → an "all synced" screen; zero taps;
  * something to delete → the deletion prompt only; one tap.

### 3.5 Statelessness

The phone persists nothing about transfer progress (only the pairing credential). Every
session re-derives its work from handshake + catalog diff; all durable state lives on the
desktop. Mid-session state — the frozen catalog, the fact that Start was already tapped —
lives in RAM only: if the app dies, the next launch is simply a new session.

## 4. Desktop app

Platform: Linux with GUI; **KDE Plasma** is the target environment.

* **Tray + window** application, XDG autostart on login — always ready, so the parents only
  ever think about the phone.
* Window contents: per-phone live progress; staged sessions with a manual **"Commit now"**
  action (rescues staging orphaned by a phone that never returns — e.g. lost, dead, or
  factory-reset, which changes its device ID; discards any in-progress partials, a
  progress-only loss, see §7.3 step 3); vault path setting (default `~/Pictures/Camera`);
  pairing/unpairing UI.
* The **vault path is a plain setting**: changing it moves nothing — the user relocates the
  vault contents themselves. Safety degrades gracefully either way: photos absent from the
  new location are simply never nominated for deletion (§8) and never re-imported (§7.5);
  staged data under the old path is abandoned and the affected files just re-transfer.
* **Inhibits system suspend** while any session is active.
* Accepts **concurrent sessions from multiple phones**; staging is isolated per device ID.
  A second live connection claiming an already-connected device ID supersedes the first.
* **Free-space check** against the diff's **to-send byte total** (§6.2) before the transfer
  begins; insufficient space produces a plain-language error on the phone. Disk exhaustion
  mid-transfer surfaces as an ordinary receive error on that file (skip + report).

## 5. Discovery, pairing, transport

### 5.1 Discovery

mDNS/DNS-SD, service type `_photosync._tcp`. Start order of the two apps is irrelevant.
The phone auto-connects to its paired desktop; if it isn't found, a friendly
"is the computer on?" screen with retry. (Assumption: the home router passes multicast)

### 5.2 Pairing (one-time) and TLS

* First connection displays a short code on the desktop screen; the parent confirms it on
  the phone. Both sides pin each other's keys; all subsequent sessions are automatic and
  TLS-encrypted.
* The phone stores exactly **one** paired desktop. A changed desktop key (e.g. reinstall)
  triggers a visible re-pair flow — never silent trust. Both sides offer "unpair".
* Handshake payload: protocol version, device ID, device name.

**Threat justification.** Without authentication, a LAN attacker could impersonate the
receiver — capturing photos, and worse: accepting the streams, confirming receipt, then
nominating for deletion the very files it received and discarded. **Pairing blocks receiver
impersonation outright.** The hash-gated deletion protocol (§8) *independently* blocks the
other attack class — forging desktop state (index or staging entries) without holding the
content, since a forger cannot produce the sha256 of a photo it never received. The two
mechanisms are complementary, not redundant: state forgery is blocked twice over; receiver
impersonation is blocked by pairing alone.

## 6. Session flow

1. **Catalog.** Phone → frozen list of `(path, size, mtime)` + total bytes. Desktop acks.
2. **Diff.** Desktop partitions the catalog:
   * **previously imported** — matches this device's index row on (path, size, mtime);
   * **already staged** — matches a verified staging-manifest entry on (path, size, mtime);
   * **to send** — everything else.
   Desktop checks free space against the to-send byte total (shortfall → plain-language
   error on the phone), then sends the to-send list, including a **resume offset** for any
   file with a partial `.part` in staging (§7.6). An entry whose (size, mtime) no longer
   matches the snapshot taken when its partial began is treated as changed on the phone —
   the partial is discarded and the file restarts from zero.
3. **Confirm.** Phone shows the summary; user taps Start. An empty to-send list skips
   straight to step 6, tap-free (§3.4).
4. **Transfer.** Multiple concurrent file streams to utilize full bandwidth. Each file is
   streamed with its sha256 (computed on the fly); the catalog mtime accompanies the file
   and is applied to the stored copy. A file that changed on the phone mid-session
   (size/mtime mismatch at send time) is skipped and reported; the desktop discards any
   partial it holds for it.
5. **Receive & verify.** Desktop writes each stream into staging under a
   **manifest-assigned unique name** (`<id>.part` — the manifest maps entries to staging
   files, so a re-transfer of a changed file can never collide with an older staged version:
   the older entry is superseded and its file removed). It fsyncs, verifies the sha256
   against the phone's digest, atomically renames to strip the suffix, and records
   `(path, size, mtime, sha256)` as a **verified** manifest entry.
   * Hash mismatch → delete the partial, one full re-transfer, then skip + report.
   * A stream is bounded by the size its catalog entry declared. Bytes past that size are
     refused and the file is reported as a receive error, keeping its partial for a later
     resume. The catalog is the only statement of how large a file is, so a desktop that
     kept reading would let one phone fill the staging filesystem.
   * Resume of partial files follows §7.6.
   * **All verified files are staged, duplicate content included.** Deduplication happens
     at exactly one point: under the commit lock (§7.3 step 1). (A receive-time index
     check was considered and rejected: it would duplicate the dedup logic and write to
     the index outside the commit lock for no behavioral difference; staging a duplicate
     merely costs its bytes some staging space until the finish signal.)
6. **Finish.** Phone → finish signal (sent automatically when there was nothing to send).
   Desktop commits (§7), then derives and sends the deletion-candidate list (§8). Phone
   prompts, deletes, reports per-file results. Both ends show a session summary:
   sent / skipped / failed / deleted / kept.

**Reconnection (same code path).** A connection drop mid-session is rejoined by re-running
the handshake and re-sending the *frozen* catalog; the desktop re-diffs. Files verified in
the meantime drop out via the staging manifest, and the interrupted file comes back with its
durable-watermark offset (§7.6) — the desktop's manifest is the only truth about what has
durably arrived, so the phone never resumes from its own send counter. Because the catalog
is frozen, the re-diff can only shrink the work, so the phone skips Confirm and continues
silently: sending photo *n+1* exercises exactly the code path that sent photo 1. There is
no session token — skipping Confirm is the phone's own RAM-state decision, and every
connection is authenticated by the pinned TLS identity. Interrupted sessions therefore need
no special handling: staging and its manifest persist, and the next diff — seconds later or
next week — resumes exactly where things stopped.

## 7. Staging, commit, vault

### 7.1 Layout

* Staging lives at **`<vault>/.staging/<deviceID>/`** — same filesystem as the vault, so
  commit is an atomic `rename()` per file: near-instant, no transient 2× disk usage.
* Staging files are named by manifest-assigned IDs (§6.5), not phone filenames.
* The vault itself is a **flat directory**.

### 7.2 Naming

* Format: **`yyyy-MM-dd_HHmmss[_n].<ext>`** — 24-hour clock, no colons (filesystem-portable),
  e.g. `2026-09-01_123456.jpg`. `_n` (n ≥ 1) resolves collisions against existing vault
  names and within the batch. Extensions are preserved and lowercased.
* Timestamp source, in order:
  1. Photos: EXIF `DateTimeOriginal` (local wall-clock, used as-is).
  2. Videos: container `creation_time` — stored in **UTC** — converted to the desktop's
     local timezone.
  3. File mtime (carried over from the phone).
  4. Terminal fallback: the import time (desktop clock at commit).
  * Sanity check: a year < 2000 or in the future falls through to the next source.
* Naming is cosmetic. Photos use the camera's wall-clock while videos convert UTC to the
  desktop's timezone, so simultaneous captures can sort differently when the phone was in
  another timezone; correctness (dedup, deletion) never depends on names.

### 7.3 Commit procedure

Commits are **serialized across devices** (transfers stay concurrent) under one global
commit lock. A commit's **batch** is the set of verified entries in that device's staging
manifest — including leftovers staged by earlier interrupted sessions — frozen at the moment
the finish signal is processed: the per-device lock is held from finish-signal (commit
enqueued) until the commit completes, so a phone that immediately reconnects is held with a
"finishing previous import…" status and can never add files to a queued or running batch
(the wait is short, since commit = renames).

Batches are disjoint across devices by construction (per-device staging), so queued commits
never inspect one another; cross-batch duplicates resolve transitively through the index,
because each commit inserts its index rows *before* releasing the global commit lock
(step 3), and the next commit's map building (step 1) therefore sees them.

1. **Build the name map.** For each verified manifest entry, look up its recorded sha256
   (computed during streaming — no file bytes are re-read; commits stay pure metadata work)
   in the index **under the commit lock** — the single authoritative dedup point, which also
   closes the race where two concurrent sessions staged the same content before either
   committed. Map entries are either **import** (staged file → final vault name) or
   **duplicate** (hash already in the index, or already assigned earlier in this batch,
   tracked via a hash set while building the map → no vault write; index device-entry only,
   inheriting the vault name already assigned to that content). A manifest entry whose
   staging file is missing is dropped with a logged warning. Write the map to a
   **write-log** in the staging directory; flush; seal with an end marker.
2. **Execute, in groups.** For each entry: **import** → `rename()` into the vault;
   **duplicate** → delete the staged file. After each group, fsync the vault and staging
   directories, **then** append flushed done-marks for the group. The directory fsync must
   precede the done-marks: a done-mark that survives a power loss must imply its rename did
   too — the same buffered-durability trap §7.6 guards against for file data, applied to
   directory metadata. A failed `rename()` (ENOSPC, I/O error) halts the commit after the
   current entry; the sealed log plus recovery replay (§7.4) completes it once the condition
   clears, and the error is surfaced in the desktop UI.
3. **Finalize.** Insert index rows for the batch (durable before proceeding), then clear the
   entire device staging directory in this order: **write-log first** (recovery then treats
   staging as fresh; any manifest rows for already-imported content resolve as duplicates at
   the next commit), then the manifest, then remaining files. Clearing removes in-progress
   partials too — a deliberate simplification: a partial is never the sole copy of anything
   (the phone still holds the file, and deletion only ever targets index-covered files), so
   the worst case is a re-transfer from zero. The same applies to the manual
   **"Commit now"** (§4).

### 7.4 Crash recovery (idempotent at any power-off point)

* Absent or unsealed write-log → discard it; staging is not mid-commit. Stale manifest rows,
  if any, resolve at the next commit (step 1 drops rows without files; step 3's clear order
  guarantees index rows became durable before the log was removed).
* Sealed write-log → re-execute entries lacking done-marks. The sealed map reserved each
  target name, so a file already present at a target path is our own completed rename;
  re-executing a **duplicate** entry just deletes the staged file if still present.
* Replay index insertion (idempotent upsert — the manifest supplies the row data, which is
  why the clear order deletes it *after* the write-log), then clear staging as in step 3.
* Recovery assumes the vault has a **single writer**: nothing else creates files there
  between crash and recovery. ("Already present at the target ⇒ our own rename" is sound
  only under that assumption.)

### 7.5 Import index

SQLite (WAL mode, synchronous writes), stored in the desktop app's data directory —
deliberately outside the vault, so vault relocation or curation never touches it:

* **content** table keyed by sha256;
* **device_file** table: `(device_id, device_path, size, mtime, sha256, vault_name, committed_at)`,
  keyed by **(device_id, device_path)** — re-importing a changed file **replaces** the row,
  mirroring the phone: an edited photo replaced the original there, so its record replaces
  the original here.

Records survive vault curation: once imported, a photo is never re-imported — even if the
user later deletes or moves the vault copy (deletion nomination then just skips it, §8).

**Failure analysis.** *Lost* index rows cause duplicate re-imports — annoying, harmless.
The subtler worst case is a *plausibly wrong* row (corruption, or forgery per §5.2) that
matches a real file's (path, size, mtime): that file is silently never backed up — the diff
forever classifies it as imported, while the deletion hash-gate correctly keeps it. It can
never be wrongly deleted, but it is stranded. **Escape hatch:** the phone reports per-file
keep results (§6.6); a file kept as "changed since backup" across repeated sessions without
ever being re-streamed is flagged in the desktop UI with a **force re-import** action
(drops the stale row so the next diff re-sends it).

### 7.6 Partial files and resume

* When a file's transfer begins, the staging manifest records an in-progress entry: the
  catalog snapshot `(path, size, mtime)`, the staging filename, and a **durable watermark**
  `durable_bytes = 0`.
* During receive, the desktop fsyncs the `.part` file periodically (e.g. every 16 MB) and
  after each fsync advances the watermark in the manifest (flushed). On connection loss it
  fsyncs once more and finalizes the watermark.
* On desktop startup after a crash, every `.part` file is **truncated to its watermark** —
  the prefix is then exactly the bytes known to be durable; a torn or zero-filled tail from
  a power loss can never survive into a resumed file. (Plain file size is *not* a safe
  offset: a crashed buffered write can extend the file past its durable data.)
* The resume offset reported in the diff (§6.2) is the watermark. On resume, the desktop
  replays the existing prefix through its sha256 state and continues with the streamed
  remainder; the phone re-reads the whole file locally to compute its digest (cheap local
  I/O) but transmits only bytes ≥ offset. The end-to-end digest comparison therefore covers
  the prefix too — resume is purely an optimization, never a trust assumption. A final
  mismatch falls back to the standard policy: delete the partial, one full re-transfer,
  then skip + report.

## 8. Deletion protocol — desktop nominates, phone proves

* After commit, the desktop derives deletion candidates **only from the current session's
  catalog** (so the list can never reference files the phone no longer knows): every catalog
  entry covered by the index **whose vault copy is verified present** — a `stat()` of the
  recorded vault name checking existence and size, at nomination time. A curated photo
  (vault copy deleted or moved by the user) is thus silently kept on the phone: the vault,
  not the index, is the source of truth for "safely stored". Files committed this session
  trivially pass; the check really guards previously-imported candidates.
  Each candidate: `(device_path, expected_sha256, size, mtime)`.
* The phone shows **one prompt** with the split:
  "3,214 photos are safely stored on the computer — 1,180 from this transfer, 2,034 from
  earlier. Free up 18.2 GB?" — "from this transfer" means committed by this session's batch
  (whether streamed just now or staged by an earlier interrupted session); "from earlier"
  means matched against pre-existing index rows.
* On confirm, before each individual delete:
  * candidate **committed this session** → verify size + mtime unchanged. (Its
    (path, size, mtime) was matched against a hash-verified staging entry during this
    session's diff or receive, so the cheap check carries the full digest guarantee.)
  * **previously-imported** candidate → full local re-hash vs `expected_sha256`
    (hundreds of MB/s on-device; progress shown — a mostly once-ever backlog cost);
  * any mismatch → **keep the file**, report ("3 photos changed since backup and were kept").
    A genuinely edited photo is re-imported as a new file next session via the diff; a file
    kept repeatedly *without* ever re-importing indicates a stale index row — see §7.5's
    escape hatch.
* Deletion uses `MediaStore.createDeleteRequest`: **silent** when `MANAGE_MEDIA` is granted;
  graceful fallback is the system confirmation dialog, batched in chunks (binder transaction
  size limits). `createTrashRequest` was rejected as the primary mechanism: same per-request
  consent, and trashed items keep occupying storage for up to ~30 days — defeating the
  space-freeing goal.
* **Failure analysis:** desktop state (index or staging), however stale, corrupted, or
  forged, can only cause *under-deletion* — never a wrong deletion. The this-session gate is
  anchored in content the desktop hash-verified and committed within this session; the
  earlier-session gate requires corruption or an attacker to predict the sha256 of content
  it never held.

## 9. Out of scope

Vault backup, cloud sync, gallery/browsing features, AP-isolation discovery fallback,
non-camera buckets (screenshots, app downloads), iOS. Also explicitly out of scope:

* **Vault relocation tooling** — the path is a plain setting; the user moves files (§4).
* **In-place edits that preserve (size, mtime)** — an edit landing within mtime granularity
  at identical size is invisible to the diff and to the this-session deletion gate; the
  originally captured bytes are already archived, so the exposure is the edit, not the photo.
* **Rename detection** — a file renamed on the phone re-streams in full and dedups only at
  commit: correct, but costs one full transfer. A content-hint in the catalog is a possible
  v2 optimization.

---

## Appendix A — Decision log

| # | Decision |
|---|---|
| R1 | Persistent import index (dedup memory survives commits and vault curation) |
| R2 | One-time pairing code + pinned TLS |
| R3 | Play-Store-compliant permission model (no `MANAGE_EXTERNAL_STORAGE`) |
| R4 | Flat vault directory |
| R5 | `MANAGE_MEDIA` silent delete; dialog fallback |
| R6 | Catalog scope: `DCIM/Camera` bucket only |
| R7 | Desktop: autostart tray + window |
| R8 | Target phones: Xiaomi/Redmi/POCO + Oppo/Vivo/OnePlus/realme |
| R9 | Languages: zh-CN + en |
| R10 | Commit by rename; staging inside the vault filesystem |
| R11 | Cross-device duplicate content: skip, count as synced |
| R12 | Delete prompt covers previously-imported photos too |
| R13 | Desktop environment: KDE Plasma |
| R14 | Deletion nomination verifies the vault copy exists (curation ⇒ photo stays on the phone) |
| R15 | Reconnect re-sends the frozen catalog and re-diffs; Confirm skipped phone-side; no session token |
| R16 | Single dedup point under the commit lock (no receive-time dedup) |
| R17 | Staging files named by manifest IDs; re-transfer of a changed path supersedes the older staged entry |
| R18 | Free-space check tests the diff's to-send total, not the catalog total |
| R19 | Empty to-send ⇒ auto-finish: 0–1-tap sessions, leftovers still commit |
| R20 | `device_file` keyed by (device_id, device_path); re-import replaces the row |
| R21 | Directory-fsync barriers before done-marks; staging cleared log → manifest → files |
| R22 | Repeated "kept" without re-import flags a stale index row; force re-import escape hatch |
| R23 | Commit clears the whole device staging dir; partials are progress-only and may be discarded |
| R24 | Vault path is a plain setting (no migration); index lives in the app data directory |
| R25 | A stream is bounded by its declared catalog size; an overrun is an ordinary receive error |
