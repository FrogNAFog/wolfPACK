package net.wardriver.mobile.policy

import net.wardriver.mobile.NativeCore
import net.wardriver.mobile.policy.snifferIdAsWireU32
import org.json.JSONArray
import org.json.JSONObject

/** Builds PackLink `ChannelPlanSyncV1` wire bytes for gateway SoftAP write. */
object ChannelPlanSyncBuilder {
    data class PinEntry(
        val snifferId: Int,
        val channel: Int,
    )

    /**
     * Packed via Rust `packChannelPlanSyncJson` JNI (mirrors `wardriver-packlink`).
     */
    fun buildWireBytes(
        revision: Int,
        dwellMs: Int,
        include24Ghz: Boolean = true,
        include5Ghz: Boolean = true,
        includeDfs: Boolean = true,
        bandMode: String = "auto",
        channels24: List<Int> = emptyList(),
        channels5: List<Int> = emptyList(),
        channels5Dfs: List<Int> = emptyList(),
        sniffers24: List<Int> = emptyList(),
        sniffers5: List<Int> = emptyList(),
        pins: List<PinEntry> = emptyList(),
        seq: Int = 0,
    ): ByteArray? {
        val root =
            JSONObject().apply {
                put("revision", revision)
                put("include_2_4_ghz", include24Ghz)
                put("include_5_ghz", include5Ghz)
                put("include_dfs", includeDfs)
                put("dwell_ms", dwellMs.coerceIn(50, 10_000))
                put("band_mode", bandMode)
                put("channels_2_4", JSONArray(channels24.distinct().sorted().take(32)))
                put("channels_5", JSONArray(channels5.distinct().sorted().take(32)))
                put("channels_5_dfs", JSONArray(channels5Dfs.distinct().sorted().take(32)))
                put(
                    "sniffers_2_4",
                    JSONArray().apply {
                        sniffers24.distinct().take(16).forEach { put(snifferIdAsWireU32(it)) }
                    },
                )
                put(
                    "sniffers_5",
                    JSONArray().apply {
                        sniffers5.distinct().take(16).forEach { put(snifferIdAsWireU32(it)) }
                    },
                )
                put(
                    "pins",
                    JSONArray().apply {
                        pins.take(16).forEach { pin ->
                            put(
                                JSONObject().apply {
                                    put("sniffer_id", snifferIdAsWireU32(pin.snifferId))
                                    put("channel", pin.channel.coerceAtLeast(0))
                                },
                            )
                        }
                    },
                )
                put("seq", seq)
            }
        return NativeCore.packChannelPlanSync(root.toString())
    }
}
