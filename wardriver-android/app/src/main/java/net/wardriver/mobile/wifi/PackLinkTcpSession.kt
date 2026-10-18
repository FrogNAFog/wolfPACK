package net.wardriver.mobile.wifi

import java.io.InputStream
import java.io.OutputStream
import java.util.zip.CRC32

/** Plaintext PackLink PLNK frames over a gateway SoftAP TCP socket. */
class PackLinkTcpSession(
    private val input: InputStream,
    private val output: OutputStream,
) {
    fun writeFrame(frame: ByteArray) {
        output.write(frame)
    }

    fun readFrame(): ByteArray = readPlnkFrame(input)

    companion object {
        private const val PLNK_HEADER_LEN = 14
        private const val CRC_LEN = 4

        fun packPhoneHello(phoneId: String, seq: Int = 1): ByteArray {
            val idBytes = phoneId.toByteArray(Charsets.US_ASCII)
            require(idBytes.size in 1..36)
            val payload = ByteArray(1 + idBytes.size)
            payload[0] = idBytes.size.toByte()
            idBytes.copyInto(payload, 1)
            return packFrame(msgType = 11, flags = 0, seq = seq, payload = payload)
        }

        fun packTrustForget(seq: Int = 1): ByteArray =
            packFrame(msgType = 15, flags = 0, seq = seq, payload = ByteArray(0))

        fun packSoftApConfig(ssid: String, password: String, seq: Int = 1): ByteArray {
            val ssidBytes = ssid.toByteArray(Charsets.UTF_8)
            val passBytes = password.toByteArray(Charsets.UTF_8)
            require(ssidBytes.size in 1..32)
            require(passBytes.size in 8..63)
            val payload =
                ByteArray(2 + ssidBytes.size + passBytes.size).also { out ->
                    var off = 0
                    out[off++] = ssidBytes.size.toByte()
                    ssidBytes.copyInto(out, off)
                    off += ssidBytes.size
                    out[off++] = passBytes.size.toByte()
                    passBytes.copyInto(out, off)
                }
            return packFrame(msgType = 14, flags = 0, seq = seq, payload = payload)
        }

        private fun packFrame(msgType: Int, flags: Int, seq: Int, payload: ByteArray): ByteArray {
            val headerLen = 14
            val out = ByteArray(headerLen + payload.size + CRC_LEN)
            out[0] = 'P'.code.toByte()
            out[1] = 'L'.code.toByte()
            out[2] = 'N'.code.toByte()
            out[3] = 'K'.code.toByte()
            out[4] = 1
            out[5] = msgType.toByte()
            out[6] = flags.toByte()
            out[7] = 0
            out[8] = (payload.size and 0xFF).toByte()
            out[9] = ((payload.size shr 8) and 0xFF).toByte()
            out[10] = (seq and 0xFF).toByte()
            out[11] = ((seq shr 8) and 0xFF).toByte()
            out[12] = ((seq shr 16) and 0xFF).toByte()
            out[13] = ((seq shr 24) and 0xFF).toByte()
            payload.copyInto(out, headerLen)
            val crc = crc32(out.copyOf(headerLen + payload.size))
            val crcOff = headerLen + payload.size
            out[crcOff] = (crc and 0xFF).toByte()
            out[crcOff + 1] = ((crc shr 8) and 0xFF).toByte()
            out[crcOff + 2] = ((crc shr 16) and 0xFF).toByte()
            out[crcOff + 3] = ((crc shr 24) and 0xFF).toByte()
            return out
        }

        private fun crc32(data: ByteArray): Int {
            val crc = CRC32()
            crc.update(data)
            return crc.value.toInt()
        }

        fun readPlnkFrame(input: InputStream): ByteArray {
            val header = input.readNBytes(PLNK_HEADER_LEN)
            if (header.size < PLNK_HEADER_LEN) {
                throw IllegalStateException("TCP closed during PLNK header")
            }
            if (header[0] != 'P'.code.toByte() || header[1] != 'L'.code.toByte() ||
                header[2] != 'N'.code.toByte() || header[3] != 'K'.code.toByte()
            ) {
                throw IllegalStateException("bad PackLink magic")
            }
            val payloadLen =
                (header[8].toInt() and 0xFF) or ((header[9].toInt() and 0xFF) shl 8)
            val tail = input.readNBytes(payloadLen + CRC_LEN)
            if (tail.size < payloadLen + CRC_LEN) {
                throw IllegalStateException("truncated PLNK frame")
            }
            return header + tail
        }
    }
}

private fun InputStream.readNBytes(count: Int): ByteArray {
    val buf = ByteArray(count)
    var off = 0
    while (off < count) {
        val n = read(buf, off, count - off)
        if (n <= 0) break
        off += n
    }
    return if (off == count) buf else buf.copyOf(off)
}
