package top.wxx9248.photosync.session.verification

/**
 * Declares which requirements a test covers.
 *
 * The runner reads these declarations from the sources to build the traceability matrix, so the
 * annotation itself carries no behavior. Identifiers come from `verification/requirements.toml`.
 */
@Retention(AnnotationRetention.SOURCE)
@Target(AnnotationTarget.FUNCTION, AnnotationTarget.CLASS)
annotation class Covers(vararg val ids: String)
