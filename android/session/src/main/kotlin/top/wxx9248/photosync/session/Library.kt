package top.wxx9248.photosync.session

/**
 * The photographs this phone holds, as the session needs to see them.
 *
 * Everything the state machine knows about storage is here, and none of it mentions Android.
 * The real implementation reads the media store; a test hands over a map. That is rule K13 of
 * `docs/CONVENTIONS-KOTLIN.md` and it is what lets the whole of §6 be checked on a plain JVM.
 *
 * Reading only. §8 removes photographs through a request the platform puts to a person, which
 * is neither synchronous nor something a state machine can do, so the session decides what may
 * go and hands the list to whoever is driving it.
 */
interface Library {
    /** What the file is right now, or null if it is gone. */
    fun describe(path: DevicePath): CatalogEntry?

    /** Part of a file. A caller asking past the end gets what there is. */
    fun read(path: DevicePath, offset: Long, length: Int): ByteArray

    /**
     * The sha256 of the whole file, prefix included.
     *
     * §7.6 has the phone compute this over everything, including bytes a resumed transfer
     * will not send, so that a resume proves as much as a fresh transfer does.
     */
    fun digest(path: DevicePath): Sha256
}
