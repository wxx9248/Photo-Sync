package top.wxx9248.photosync.session

import java.nio.file.Path
import kotlin.io.path.readLines
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue
import top.wxx9248.photosync.session.verification.Covers

/**
 * The shared conformance vectors of `verification/vectors/`.
 *
 * The deletion gates are the one decision made on the phone that can destroy something, so
 * the cases are written down once and both implementations answer them. A disagreement here
 * is not a failing test on one end; it is the two halves of the protocol having drifted.
 */
class VectorsTest {
    private fun read(name: String): List<Map<String, String>> {
        // The module sits two directories below the repository root.
        val file = Path.of("..", "..", "verification", "vectors", name)
        val lines = file.readLines().filterNot { it.startsWith("#") || it.isBlank() }
        val columns = lines.first().split("\t")
        return lines.drop(1).map { line -> columns.zip(line.split("\t")).toMap() }
    }

    @Test
    @Covers("R-PAIR-001")
    fun `the pairing code is the one both screens have to show`() {
        val cases = read("pairing.tsv")
        assertTrue(cases.size >= 4, "the vectors were not read: ${cases.size} cases")

        for (case in cases) {
            assertEquals(
                case.getValue("code"),
                PairingCode.of(
                    hex(case.getValue("desktop_key")),
                    hex(case.getValue("phone_key")),
                ),
                case.getValue("name"),
            )
        }
    }

    private fun hex(text: String): ByteArray =
        if (text.isEmpty()) {
            ByteArray(0)
        } else {
            text.chunked(2).map { it.toInt(16).toByte() }.toByteArray()
        }

    @Test
    @Covers("R-DELETE-006", "R-DELETE-007", "R-DELETE-008")
    fun `every shared case is decided the way it is written down`() {
        val cases = read("deletion.tsv")
        assertTrue(cases.size >= 6, "the vectors were not read: ${cases.size} cases")

        for (case in cases) {
            val path = DevicePath("DCIM/Camera/IMG_0001.jpg")
            val library = FakeLibrary()
            if (case.getValue("local_present") == "yes") {
                library.put(
                    path.value,
                    case.getValue("local_mtime").toLong(),
                    case.getValue("local_content").toByteArray(),
                )
            }

            val candidate = DeletionCandidate(
                path = path,
                size = case.getValue("candidate_size").toLong(),
                mtime = Timestamp(case.getValue("candidate_mtime").toLong()),
                expected = digestOf(case.getValue("vault_content").toByteArray()),
                origin = when (case.getValue("origin")) {
                    "this-transfer" -> CandidateOrigin.THIS_TRANSFER
                    "earlier" -> CandidateOrigin.EARLIER
                    else -> error("${case["name"]}: no such origin")
                },
            )

            val expected = when (case.getValue("expect")) {
                "deleted" -> DeletionResult.DELETED
                "kept-changed" -> DeletionResult.KEPT_CHANGED
                "failed" -> DeletionResult.FAILED
                else -> error("${case["name"]}: no such outcome")
            }

            assertEquals(expected, decide(candidate, library), case.getValue("name"))
        }
    }
}
