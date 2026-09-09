package top.wxx9248.photosync.pairing

import java.io.File
import top.wxx9248.photosync.session.PairedDesktop
import top.wxx9248.photosync.session.PairingStorage

/**
 * The paired desktop, kept in the application's own storage.
 *
 * `SPEC.md` §3.5 is explicit that the pairing credential is the *only* thing the phone keeps
 * between sessions: no transfer progress, no catalog, nothing that could disagree with the
 * desktop later. Two lines of text is the whole of it.
 */
class FilePairingStorage(private val file: File) : PairingStorage {
    override fun read(): PairedDesktop? {
        if (!file.isFile) return null
        val lines = runCatching { file.readLines() }.getOrDefault(emptyList())
        if (lines.size < 2) return null
        return PairedDesktop(name = lines[0], publicKey = lines[1])
    }

    override fun write(desktop: PairedDesktop) {
        file.parentFile?.mkdirs()
        file.writeText("${desktop.name}\n${desktop.publicKey}\n")
    }

    override fun clear() {
        file.delete()
    }
}
