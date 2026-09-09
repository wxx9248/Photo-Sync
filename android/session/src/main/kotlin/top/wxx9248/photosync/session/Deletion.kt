package top.wxx9248.photosync.session

/**
 * The two gates a photograph passes before the phone lets go of it. `SPEC.md` §8.
 *
 * The desktop nominates and the phone proves. Which proof is demanded depends on what vouches
 * for the copy: a photograph committed in this session had its (path, size, mtime) matched
 * against a staging entry the desktop hash-verified minutes ago, so the cheap check carries
 * the full guarantee. One matched against an older index row has nothing recent behind it, so
 * the phone reads the file and hashes it.
 *
 * Every uncertainty resolves the same way. A file that is gone, changed, or unreadable is
 * kept and reported: §8's failure analysis is that desktop state, however stale or forged,
 * can cause under-deletion and never a wrong one, and that only holds if this never guesses.
 */
internal fun decide(candidate: DeletionCandidate, library: Library): DeletionResult {
    val held = library.describe(candidate.path) ?: return DeletionResult.FAILED

    val unchanged = held.size == candidate.size && held.mtime == candidate.mtime
    if (!unchanged) {
        return DeletionResult.KEPT_CHANGED
    }

    val proven = when (candidate.origin) {
        CandidateOrigin.THIS_TRANSFER -> true
        CandidateOrigin.EARLIER -> library.digest(candidate.path) == candidate.expected
    }

    return if (proven) DeletionResult.DELETED else DeletionResult.KEPT_CHANGED
}
