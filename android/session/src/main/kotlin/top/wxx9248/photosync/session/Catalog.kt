package top.wxx9248.photosync.session

/**
 * One photograph as the phone sees it.
 *
 * `SPEC.md` §3.2 fixes the identity of an entry as the three fields together, and everything
 * else in the protocol rests on that: the diff matches on it, the desktop refuses an upload
 * whose header disagrees with it, and the cheap deletion gate compares it.
 */
data class CatalogEntry(
    val path: DevicePath,
    val size: Long,
    val mtime: Timestamp,
)

/**
 * Everything the phone is offering, frozen.
 *
 * §3.2 enumerates once, at the start of a session, and holds the result for the whole of it.
 * A reconnection re-sends this same list, which is what makes a re-diff able only to shrink
 * the work. Freezing it is the reason this type exists rather than a bare list: a caller
 * cannot add to it half way through.
 */
class Catalog(entries: List<CatalogEntry>) {
    val entries: List<CatalogEntry> = entries.toList()

    val totalBytes: Long = entries.sumOf { it.size }

    operator fun get(path: DevicePath): CatalogEntry? = entries.firstOrNull { it.path == path }
}
