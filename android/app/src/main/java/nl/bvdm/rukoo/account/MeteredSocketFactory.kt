package nl.bvdm.rukoo.account

import java.io.FilterInputStream
import java.io.IOException
import java.io.InputStream
import java.net.InetAddress
import java.net.InetSocketAddress
import java.net.Socket
import java.util.concurrent.atomic.AtomicLong
import javax.net.SocketFactory

/**
 * Plain sockets that count every byte read from them, so an answer's headers and HTTP framing count as well as its
 * body. Under TLS, the platform's TLS socket reads the encrypted stream through [Socket.getInputStream] of the socket
 * it wraps, so the count is the encrypted bytes, handshake included. [read] adds up every socket this factory made;
 * a read that takes it past [limit] fails with [TooMuchRead], and so does every read after it.
 */
internal class MeteredSocketFactory(val limit: Long) : SocketFactory() {
    val read = AtomicLong()

    /** Whether the sockets read past [limit]. */
    val over: Boolean get() = read.get() > limit

    // OkHttp asks for an unconnected socket and connects it itself; the other ways are here for completeness.
    override fun createSocket(): Socket = MeteredSocket()

    override fun createSocket(host: String?, port: Int): Socket = connected(InetSocketAddress(InetAddress.getByName(host), port), null)

    override fun createSocket(host: String?, port: Int, localHost: InetAddress?, localPort: Int): Socket =
        connected(InetSocketAddress(InetAddress.getByName(host), port), InetSocketAddress(localHost, localPort))

    override fun createSocket(host: InetAddress?, port: Int): Socket = connected(InetSocketAddress(host, port), null)

    override fun createSocket(address: InetAddress?, port: Int, localAddress: InetAddress?, localPort: Int): Socket =
        connected(InetSocketAddress(address, port), InetSocketAddress(localAddress, localPort))

    private fun connected(remote: InetSocketAddress, local: InetSocketAddress?): Socket = MeteredSocket().apply {
        try {
            if (local != null) bind(local)
            connect(remote)
        } catch (e: IOException) {
            close()
            throw e
        }
    }

    private fun count(n: Long) {
        if (read.addAndGet(n) > limit) throw TooMuchRead(limit)
    }

    private fun refuseWhenOver() {
        if (over) throw TooMuchRead(limit)
    }

    private inner class MeteredSocket : Socket() {
        override fun getInputStream(): InputStream = Metered(super.getInputStream())
    }

    private inner class Metered(input: InputStream) : FilterInputStream(input) {
        override fun read(): Int {
            refuseWhenOver()
            return super.read().also { if (it >= 0) count(1) }
        }

        override fun read(b: ByteArray, off: Int, len: Int): Int {
            refuseWhenOver()
            return super.read(b, off, len).also { if (it > 0) count(it.toLong()) }
        }

        override fun skip(n: Long): Long {
            refuseWhenOver()
            return super.skip(n).also { if (it > 0) count(it) }
        }
    }
}

/** An answer that took more bytes than one call may read. */
internal class TooMuchRead(limit: Long) : IOException("more than $limit bytes read")
