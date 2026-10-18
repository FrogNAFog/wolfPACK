package net.wardriver.mobile.upload

import android.os.Build
import net.wardriver.mobile.NativeCore
import org.json.JSONObject

/** WiGLE CSV v1.6 pre-header + column header (matches control `csv.rs`). */
object WigleCsvBuilder {
    private const val CSV_HEADER =
        "MAC,SSID,AuthMode,FirstSeen,Channel,Frequency,RSSI,CurrentLatitude,CurrentLongitude,AltitudeMeters,AccuracyMeters,RCOIs,MfgrId,Type"

    fun headers(appRelease: String = "0.1.0"): String {
        val model = Build.MODEL ?: "Android"
        val release = Build.VERSION.RELEASE ?: "unknown"
        val device = Build.DEVICE ?: model
        val display = Build.DISPLAY ?: release
        val board = Build.BOARD ?: "phone"
        val req =
            JSONObject()
                .put("app_release", appRelease)
                .put("model", model)
                .put("release", release)
                .put("device", device)
                .put("display", display)
                .put("board", board)
                .put("brand", "wolfPACK")
                .put("star", "Sol")
                .put("body", 3)
                .put("sub_body", 0)
        val rust = NativeCore.formatWigleHeaders(req)
        if (rust != null) return rust
        return kotlinHeaders(appRelease, model, release, device, display, board)
    }

    private fun kotlinHeaders(
        appRelease: String,
        model: String,
        release: String,
        device: String,
        display: String,
        board: String,
    ): String {
        val pre =
            "WigleWifi-1.6,appRelease=$appRelease,model=$model,release=$release," +
                "device=$device,display=$display,board=$board,brand=wolfPACK,star=Sol,body=3,subBody=0"
        return "$pre\n$CSV_HEADER\n"
    }
}
