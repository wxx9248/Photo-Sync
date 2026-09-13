package top.wxx9248.photosync.net

import io.grpc.ManagedChannel
import io.grpc.Server
import io.grpc.Status
import io.grpc.StatusException
import io.grpc.inprocess.InProcessChannelBuilder
import io.grpc.inprocess.InProcessServerBuilder
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertNull
import kotlin.time.Duration
import kotlin.time.Duration.Companion.seconds
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.awaitCancellation
import kotlinx.coroutines.cancel
import kotlinx.coroutines.cancelAndJoin
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.launch
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import photosync.v1.PhotoSyncGrpcKt
import photosync.v1.Sync as Wire
import photosync.v1.uploadResult
import top.wxx9248.photosync.session.DeviceId
import top.wxx9248.photosync.session.DevicePath
import top.wxx9248.photosync.session.FileId
import top.wxx9248.photosync.session.Inbound
import top.wxx9248.photosync.session.Outbound
import top.wxx9248.photosync.session.Sha256
import top.wxx9248.photosync.session.Timestamp
import top.wxx9248.photosync.session.UploadOutcome

/**
 * The wire adapter, against a desktop in this process.
 *
 * `GrpcDesktop` holds no Android type, so the whole of it runs on a plain JVM against an
 * in-process server. What is worth checking here is not the translation — the state machine's
 * own tests cover what is said — but the two things only a real call shows: that a file
 * travels as it is read rather than in one piece at the end, and that a call which is
 * cancelled stops instead of reporting an answer.
 */

private val FILE = FileId(7)
private val PATH = DevicePath("DCIM/Camera/IMG_0001.jpg")
private val DIGEST = Sha256("a".repeat(64))
private val PHOTO = "the bytes of one photograph".toByteArray()

/** Long enough for a call in this process, short enough that a hang fails the run. */
private val PATIENCE: Duration = 30.seconds

/** What the desktop saw, in the order it saw it. */
private enum class Piece { HEADER, DATA, TRAILER }

/**
 * A desktop that records what arrives and can be told to walk away part-way through.
 *
 * Arrivals go into a channel rather than a list so a test waits for the next one instead of
 * looking to see whether it has turned up yet. Rule 6.4.
 */
private class WatchingDesktop(
    private val answer: Wire.UploadResult.Status = Wire.UploadResult.Status.VERIFIED,
    private val endTheCallAfter: Int = Int.MAX_VALUE,
) : PhotoSyncGrpcKt.PhotoSyncCoroutineImplBase() {
    private val arrivals = Channel<Piece>(Channel.UNLIMITED)
    private val handshakes = Channel<Unit>(Channel.UNLIMITED)

    /** Never answers, so a test can cancel a call that is genuinely in flight. */
    override suspend fun handshake(request: Wire.HandshakeRequest): Wire.HandshakeResponse {
        handshakes.send(Unit)
        awaitCancellation()
    }

    override suspend fun uploadFile(requests: Flow<Wire.FileChunk>): Wire.UploadResult {
        var taken = 0
        requests.collect { chunk ->
            arrivals.send(
                when {
                    chunk.hasHeader() -> Piece.HEADER
                    chunk.hasTrailer() -> Piece.TRAILER
                    else -> Piece.DATA
                }
            )
            taken += 1
            if (taken >= endTheCallAfter) {
                throw StatusException(Status.RESOURCE_EXHAUSTED)
            }
        }
        return uploadResult { status = answer }
    }

    /** Waits for the next thing to arrive on the upload. */
    suspend fun next(): Piece = arrivals.receive()

    /** Waits until a handshake is actually being served. */
    suspend fun handshakeStarted() {
        handshakes.receive()
    }
}

/** A server and a channel to it, both in this process. */
private class Loopback(service: PhotoSyncGrpcKt.PhotoSyncCoroutineImplBase) : AutoCloseable {
    private val name = InProcessServerBuilder.generateName()
    private val server: Server =
        InProcessServerBuilder.forName(name).addService(service).build().start()

    val channel: ManagedChannel = InProcessChannelBuilder.forName(name).build()

    /**
     * A pool of connections to the same server.
     *
     * In process there is no socket behind any of them, so what this exercises is the
     * driver's own bookkeeping: which lane a file went out on, and that a lane comes back
     * when its file is answered for. What separate connections buy on a real network is
     * `STACK.md` §3.6's argument, not something a test in one process can show.
     */
    fun pool(size: Int): List<ManagedChannel> =
        List(size) { InProcessChannelBuilder.forName(name).build() }

    override fun close() {
        channel.shutdownNow()
        server.shutdownNow()
    }
}

/** One phone talking to `watching`, for as long as `body` runs. */
private fun phoning(watching: WatchingDesktop, body: suspend (GrpcDesktop) -> Unit) = runBlocking {
    withTimeout(PATIENCE) {
        Loopback(watching).use { wire ->
            val session = CoroutineScope(Job())
            try {
                GrpcDesktop(wire.pool(GrpcDesktop.POOL), session).use { desktop -> body(desktop) }
            } finally {
                session.cancel()
            }
        }
    }
}

private fun opening() =
    Outbound.OpenUpload(FILE, PATH, PHOTO.size.toLong(), Timestamp(1_756_000_000), 0)

class GrpcDesktopTest {
    @Test
    fun `the bytes of a file reach the desktop before its digest is known`() {
        val watching = WatchingDesktop()
        phoning(watching) { desktop ->
            desktop.say(opening())
            assertEquals(Piece.HEADER, watching.next())

            desktop.say(Outbound.SendChunk(FILE, 0, PHOTO))

            // Nothing has closed the upload, so the digest is still unknown, and the bytes are
            // already on the desktop. A phone that gathered the file up first would have sent
            // nothing at all by now, and could not send a photograph larger than its memory.
            assertEquals(Piece.DATA, watching.next())
        }
    }

    @Test
    fun `the digest closes the upload and the desktop's answer comes back`() {
        val watching = WatchingDesktop()
        phoning(watching) { desktop ->
            desktop.say(opening())
            desktop.say(Outbound.SendChunk(FILE, 0, PHOTO))
            assertNull(
                desktop.say(Outbound.CloseUpload(FILE, DIGEST)),
                "closing waited for the answer instead of leaving the lane to finish",
            )

            assertEquals(
                Inbound.UploadAnswered(FILE, UploadOutcome.VERIFIED),
                desktop.answered(),
            )
            assertEquals(Piece.HEADER, watching.next())
            assertEquals(Piece.DATA, watching.next())
            assertEquals(Piece.TRAILER, watching.next())
        }
    }

    @Test
    fun `a desktop that ends the call part-way is reported rather than waited for`() {
        // The call ends on the header, so the chunk after it has nowhere to go.
        val watching = WatchingDesktop(endTheCallAfter = 1)
        phoning(watching) { desktop ->
            desktop.say(opening())
            assertEquals(Piece.HEADER, watching.next())

            desktop.say(Outbound.SendChunk(FILE, 0, PHOTO))
            desktop.say(Outbound.CloseUpload(FILE, DIGEST))

            assertEquals(
                Inbound.UploadAnswered(FILE, UploadOutcome.WRITE_FAILED),
                desktop.answered(),
            )
        }
    }

    @Test
    fun `two files are open at once and each answer names its own file`() {
        val other = FileId(FILE.value + 1)
        val watching = WatchingDesktop()
        phoning(watching) { desktop ->
            desktop.say(opening())
            desktop.say(Outbound.OpenUpload(other, PATH, PHOTO.size.toLong(), Timestamp(1), 0))
            desktop.say(Outbound.SendChunk(FILE, 0, PHOTO))
            desktop.say(Outbound.SendChunk(other, 0, PHOTO))

            // Both are closed before either is asked about, which is only possible because
            // closing no longer waits. §6.4.
            desktop.say(Outbound.CloseUpload(FILE, DIGEST))
            desktop.say(Outbound.CloseUpload(other, DIGEST))

            val answers = setOf(desktop.answered(), desktop.answered())

            assertEquals(
                setOf(
                    Inbound.UploadAnswered(FILE, UploadOutcome.VERIFIED),
                    Inbound.UploadAnswered(other, UploadOutcome.VERIFIED),
                ),
                answers,
            )
            assertNull(desktop.answered(), "the driver invented an answer nobody was owed")
        }
    }

    @Test
    fun `a cancelled call stops instead of reporting a refusal`() {
        val watching = WatchingDesktop()
        phoning(watching) { desktop ->
            val answered = CompletableDeferred<Inbound?>()
            val asking = Outbound.Handshake(DeviceId("phone-a"), "Kitchen phone", 1)
            coroutineScope {
                val asked = launch { answered.complete(desktop.say(asking)) }
                watching.handshakeStarted()
                asked.cancelAndJoin()
            }

            // Rule K8. A `runCatching` here would have caught the cancellation and handed the
            // state machine a refusal, leaving a session running that was told to stop.
            assertFalse(
                answered.isCompleted,
                "a cancelled call produced an answer instead of stopping",
            )
        }
    }
}
