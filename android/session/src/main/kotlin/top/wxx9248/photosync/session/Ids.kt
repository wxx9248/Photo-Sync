package top.wxx9248.photosync.session

/**
 * A phone, as the desktop knows it.
 *
 * These are value classes because the compiler should refuse to let a display name be passed
 * where an identifier belongs, and because at runtime they cost nothing.
 */
@JvmInline
value class DeviceId(val value: String)

/** Where a photograph lives on the phone, relative to the media store's root. */
@JvmInline
value class DevicePath(val value: String)

/** The name a staging file was given by the desktop. The phone only ever echoes it back. */
@JvmInline
value class FileId(val value: Long)

/** A sha256, as the lowercase hexadecimal both ends write it. */
@JvmInline
value class Sha256(val hex: String) {
    init {
        require(hex.length == 64) { "a sha256 is 64 hexadecimal characters, not ${hex.length}" }
    }
}

/** Whole seconds since the epoch, which is the only precision `SPEC.md` §3.2 relies on. */
@JvmInline
value class Timestamp(val seconds: Long)
