package top.wxx9248.photosync.net

import android.util.Log
import io.grpc.ManagedChannel
import io.grpc.okhttp.OkHttpChannelBuilder
import java.net.InetSocketAddress
import java.util.concurrent.TimeUnit
import java.security.cert.CertificateException
import java.security.cert.X509Certificate
import javax.net.ssl.SSLContext
import javax.net.ssl.X509TrustManager
import kotlin.coroutines.coroutineContext
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Deferred
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.async
import kotlinx.coroutines.cancel
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.plus
import photosync.v1.PairingGrpcKt
import photosync.v1.pairRequest
import top.wxx9248.photosync.session.DeviceId
import top.wxx9248.photosync.session.PairedDesktop
import top.wxx9248.photosync.session.PairingCode
import top.wxx9248.photosync.session.Session

/**
 * Meeting a desktop for the first time. `SPEC.md` §5.2.
 *
 * This is the one connection the phone opens without knowing who is on the other end, so it is
 * also the only place a certificate is accepted unseen. What makes that safe is not the
 * transport but the person: both ends derive six digits from the two keys they now hold, the
 * two screens show them, and somebody who can see both says whether they match. A man in the
 * middle holds a different key on each side and cannot make the two screens agree.
 *
 * Nothing is remembered here. The key is handed back and [top.wxx9248.photosync.session.Pairing]
 * decides what to do with it, because replacing a desktop already known is a decision that
 * belongs with the rest of the pairing rules rather than with the socket.
 */
internal class GrpcPairing private constructor(
    private val channel: ManagedChannel,
    private val identity: Identity,
    private val presented: CompletableDeferred<X509Certificate>,
    parent: CoroutineScope,
) : AutoCloseable {
    private val stub = PairingGrpcKt.PairingCoroutineStub(channel)

    /**
     * The call runs while the code is on screen, so it outlives the function that starts it:
     * the desktop does not answer until a person there has decided.
     */
    private val asking = parent + SupervisorJob(parent.coroutineContext[Job])

    private var answer: Deferred<PairedDesktop?>? = null

    /**
     * Asks, and hands back the six digits to put on screen.
     *
     * The digits are ready as soon as the handshake is, which is well before the desktop
     * answers: §5.2 has a person compare two screens, and they cannot compare what is not yet
     * shown.
     */
    suspend fun offer(device: DeviceId, name: String): String? {
        val asked = asking.async {
            try {
                pair(device, name)
            } catch (failure: Exception) {
                // A call that falls over before the handshake means no certificate is coming.
                // Without this the wait below never ends: the failure would sit unobserved in
                // the deferred while a person watched "looking for the computer" forever.
                presented.completeExceptionally(failure)
                throw failure
            }
        }
        answer = asked

        // The certificate arrives with the handshake, which the call above starts.
        val desktop = try {
            presented.await()
        } catch (failure: Exception) {
            coroutineContext.ensureActive()
            Log.w(TAG, "the desktop would not talk to an unknown phone", failure)
            return null
        }
        return PairingCode.of(desktop.publicKey.encoded, identity.certificate.publicKey.encoded)
    }

    /**
     * Waits for the person at the desktop, and says how it ended.
     *
     * A refusal here means a person at the desktop said no. Anything else --- the call turned
     * away, the handshake finished and then dropped --- means the desktop is not open to a
     * phone it does not know, which sends somebody to open pairing rather than to wonder what
     * they did wrong.
     */
    suspend fun settled(): Meeting {
        val asked = answer ?: return Meeting.Refused
        return try {
            asked.await()?.let(Meeting::Paired) ?: Meeting.Refused
        } catch (failure: Exception) {
            coroutineContext.ensureActive()
            Log.w(TAG, "the desktop did not finish pairing", failure)
            // The desktop answered at the transport and then would not go on, which on this
            // protocol means one thing: it is not open to a phone it does not know. A phone
            // whose key it has never seen is turned away as the handshake finishes, and a
            // desktop nobody has opened for pairing refuses the call itself. Both send a
            // person to the same place.
            Meeting.NotOpen
        }
    }

    private suspend fun pair(device: DeviceId, name: String): PairedDesktop? {
        val answered = stub.pair(
            pairRequest {
                protocolVersion = Session.PROTOCOL_VERSION
                deviceId = device.value
                deviceName = name
            }
        )
        if (!answered.accepted) {
            return null
        }
        return PairedDesktop(
            name = answered.desktopName,
            publicKey = Identity.pinOf(presented.await()),
        )
    }

    override fun close() {
        asking.cancel()
        channel.shutdownNow()
    }

    /**
     * Accepts whichever desktop answers, and writes down what it presented.
     *
     * Deliberately trusting: this runs only while a person has opened pairing at the desktop
     * and is standing in front of both screens, and the code they compare is what decides.
     * Every connection after this one goes through [PinnedDesktop] instead.
     */
    private class RecordingDesktop(
        private val presented: CompletableDeferred<X509Certificate>,
    ) : X509TrustManager {
        override fun checkServerTrusted(chain: Array<out X509Certificate>?, authType: String?) {
            val offered = chain?.firstOrNull()
                ?: throw CertificateException("the desktop presented no certificate")
            presented.complete(offered)
        }

        override fun checkClientTrusted(chain: Array<out X509Certificate>?, authType: String?) {
            throw CertificateException("a phone is never a server")
        }

        override fun getAcceptedIssuers(): Array<X509Certificate> = emptyArray()
    }

    companion object {
        private const val TAG = "PhotoSyncPairing"

        /**
         * Opens the one connection this phone makes to a desktop it cannot yet recognise.
         *
         * The phone still presents its own key, because the desktop needs it to derive the
         * same six digits and to write down who it just met.
         */
        fun connect(
            address: InetSocketAddress,
            identity: Identity,
            scope: CoroutineScope,
        ): GrpcPairing {
            val presented = CompletableDeferred<X509Certificate>()
            val context = SSLContext.getInstance("TLSv1.3").apply {
                init(
                    arrayOf(DeviceKeyManager(identity)),
                    arrayOf(RecordingDesktop(presented)),
                    null,
                )
            }
            val channel = OkHttpChannelBuilder.forAddress(address.hostString, address.port)
                .useTransportSecurity()
                .sslSocketFactory(context.socketFactory)
                // A desktop that goes away mid-transfer takes its socket with it and says
                // nothing. Without these the phone waits on that socket for as long as the
                // operating system lets it, which is minutes, and §6's rejoin never starts
                // because the session it would rejoin has not ended.
                .keepAliveTime(GrpcDesktop.KEEPALIVE_SECONDS, TimeUnit.SECONDS)
                .keepAliveTimeout(GrpcDesktop.KEEPALIVE_TIMEOUT_SECONDS, TimeUnit.SECONDS)
                .overrideAuthority("photo-sync")
                .build()
            return GrpcPairing(channel, identity, presented, scope)
        }
    }
}
