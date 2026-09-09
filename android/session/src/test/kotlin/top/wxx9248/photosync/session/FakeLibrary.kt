package top.wxx9248.photosync.session

import java.security.MessageDigest

/** A phone's photographs, held in memory. */
class FakeLibrary(initial: Map<String, Pair<Long, ByteArray>> = emptyMap()) : Library {
    private val files = linkedMapOf<DevicePath, Pair<Long, ByteArray>>()

    init {
        initial.forEach { (path, held) -> files[DevicePath(path)] = held }
    }

    val paths: List<DevicePath> get() = files.keys.toList()

    fun put(path: String, mtime: Long, content: ByteArray) {
        files[DevicePath(path)] = mtime to content
    }

    fun contains(path: String): Boolean = files.containsKey(DevicePath(path))

    fun catalog(): Catalog = Catalog(files.map { (path, held) ->
        CatalogEntry(path, held.second.size.toLong(), Timestamp(held.first))
    })

    override fun describe(path: DevicePath): CatalogEntry? = files[path]?.let { (mtime, bytes) ->
        CatalogEntry(path, bytes.size.toLong(), Timestamp(mtime))
    }

    override fun read(path: DevicePath, offset: Long, length: Int): ByteArray {
        val bytes = files[path]?.second ?: return ByteArray(0)
        val from = offset.coerceAtMost(bytes.size.toLong()).toInt()
        val to = (from + length).coerceAtMost(bytes.size)
        return bytes.copyOfRange(from, to)
    }

    override fun digest(path: DevicePath): Sha256 =
        digestOf(files[path]?.second ?: ByteArray(0))

    override fun delete(path: DevicePath): Boolean = files.remove(path) != null
}

fun digestOf(bytes: ByteArray): Sha256 =
    Sha256(MessageDigest.getInstance("SHA-256").digest(bytes).joinToString("") { "%02x".format(it) })
