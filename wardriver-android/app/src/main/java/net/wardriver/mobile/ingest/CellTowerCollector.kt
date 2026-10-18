package net.wardriver.mobile.ingest

import android.Manifest
import android.annotation.SuppressLint
import android.content.Context
import android.content.pm.PackageManager
import android.os.Build
import android.telephony.CellIdentityCdma
import android.telephony.CellIdentityGsm
import android.telephony.CellIdentityLte
import android.telephony.CellIdentityNr
import android.telephony.CellIdentityWcdma
import android.telephony.CellInfo
import android.telephony.CellInfoCdma
import android.telephony.CellInfoGsm
import android.telephony.CellInfoLte
import android.telephony.CellInfoNr
import android.telephony.CellInfoWcdma
import android.telephony.CellSignalStrength
import android.telephony.CellSignalStrengthGsm
import android.telephony.TelephonyManager
import androidx.core.content.ContextCompat
import org.json.JSONObject

/**
 * Phone-sourced cell tower scan (WiGLE `recordCellInfo` / `getAllCellInfo` pattern).
 *
 * Parses serving + neighbor cells into WiGLE CSV v1.6 field shapes for Rust enrichment.
 */
class CellTowerCollector(private val context: Context) {
    data class CellObservation(
        val cellKey: String,
        val networkName: String,
        val capabilities: String,
        val signalKind: String,
        val frequencyMhz: Int,
        val rssi: Int,
    )

    @SuppressLint("MissingPermission")
    fun collect(operatorName: String?): List<CellObservation> {
        if (!hasCellPermissions()) return emptyList()
        val tele =
            context.getSystemService(Context.TELEPHONY_SERVICE) as? TelephonyManager
                ?: return emptyList()
        val cells =
            try {
                tele.allCellInfo ?: return emptyList()
            } catch (_: SecurityException) {
                return emptyList()
            }
        val carrier = operatorName?.takeIf { it.isNotBlank() } ?: tele.networkOperatorName.orEmpty()
        return cells.mapNotNull { parseCellInfo(it, carrier) }
    }

    fun hasCellPermissions(): Boolean {
        val fine =
            ContextCompat.checkSelfPermission(
                context,
                Manifest.permission.ACCESS_FINE_LOCATION,
            ) == PackageManager.PERMISSION_GRANTED
        val phone =
            ContextCompat.checkSelfPermission(
                context,
                Manifest.permission.READ_PHONE_STATE,
            ) == PackageManager.PERMISSION_GRANTED
        return fine && phone
    }

    private fun parseCellInfo(info: CellInfo, carrier: String): CellObservation? {
        if (!info.isRegistered && Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            // Log neighbors too (WiGLE records all visible cells).
        }
        return when (info) {
            is CellInfoLte -> parseLte(info, carrier)
            is CellInfoNr -> parseNr(info, carrier)
            is CellInfoGsm -> parseGsm(info, carrier)
            is CellInfoWcdma -> parseWcdma(info, carrier)
            is CellInfoCdma -> parseCdma(info, carrier)
            else -> null
        }
    }

    private fun parseLte(info: CellInfoLte, carrier: String): CellObservation? {
        val id = info.cellIdentity as CellIdentityLte
        val mccMnc = mccMncFromIdentity(id.mccString, id.mncString) ?: return null
        val tac = validId(id.tac) ?: return null
        val ci = validId(id.ci) ?: return null
        val key = "${mccMnc}_${tac}_$ci"
        return CellObservation(
            cellKey = key,
            networkName = carrier,
            capabilities = "LTE;$mccMnc",
            signalKind = "LTE",
            frequencyMhz = validId(id.earfcn) ?: 0,
            rssi = signalDbm(info.cellSignalStrength),
        )
    }

    private fun parseNr(info: CellInfoNr, carrier: String): CellObservation? {
        val id = info.cellIdentity as CellIdentityNr
        val mccMnc = mccMncFromIdentity(id.mccString, id.mncString) ?: return null
        val tac = validId(id.tac) ?: return null
        val nci = validLongId(id.nci) ?: return null
        val key = "${mccMnc}_${tac}_$nci"
        return CellObservation(
            cellKey = key,
            networkName = carrier,
            capabilities = "NR;$mccMnc",
            signalKind = "NR",
            frequencyMhz = validId(id.nrarfcn) ?: 0,
            rssi = signalDbm(info.cellSignalStrength),
        )
    }

    private fun parseGsm(info: CellInfoGsm, carrier: String): CellObservation? {
        val id = info.cellIdentity as CellIdentityGsm
        val mccMnc = mccMncFromIdentity(id.mccString, id.mncString) ?: return null
        val lac = validId(id.lac) ?: return null
        val cid = validId(id.cid) ?: return null
        val key = "${mccMnc}_${lac}_$cid"
        val strength = info.cellSignalStrength
        val rssi =
            if (strength is CellSignalStrengthGsm) {
                gsmAsuToDbm(strength.asuLevel)
            } else {
                signalDbm(strength)
            }
        return CellObservation(
            cellKey = key,
            networkName = carrier,
            capabilities = "GSM;$mccMnc",
            signalKind = "GSM",
            frequencyMhz = validId(id.arfcn) ?: 0,
            rssi = rssi,
        )
    }

    private fun parseWcdma(info: CellInfoWcdma, carrier: String): CellObservation? {
        val id = info.cellIdentity as CellIdentityWcdma
        val mccMnc = mccMncFromIdentity(id.mccString, id.mncString) ?: return null
        val lac = validId(id.lac) ?: return null
        val cid = validId(id.cid) ?: return null
        val key = "${mccMnc}_${lac}_$cid"
        return CellObservation(
            cellKey = key,
            networkName = carrier,
            capabilities = "WCDMA;$mccMnc",
            signalKind = "WCDMA",
            frequencyMhz = validId(id.uarfcn) ?: 0,
            rssi = signalDbm(info.cellSignalStrength),
        )
    }

    private fun parseCdma(info: CellInfoCdma, carrier: String): CellObservation? {
        val id = info.cellIdentity as CellIdentityCdma
        val systemId = validId(id.systemId) ?: return null
        val networkId = validId(id.networkId) ?: return null
        val baseId = validId(id.basestationId) ?: return null
        val key = "${systemId}_${networkId}_$baseId"
        return CellObservation(
            cellKey = key,
            networkName = carrier,
            capabilities = "CDMA;$systemId",
            signalKind = "CDMA",
            frequencyMhz = 0,
            rssi = signalDbm(info.cellSignalStrength),
        )
    }

    private fun mccMncFromIdentity(mcc: String?, mnc: String?): String? {
        if (mcc.isNullOrBlank() || mnc.isNullOrBlank()) return null
        val mc = mcc.filter { it.isDigit() }.padStart(3, '0').takeLast(3)
        val mnRaw = mnc.filter { it.isDigit() }
        if (mc.length != 3 || mnRaw.isEmpty()) return null
        val mn =
            when (mnRaw.length) {
                2 -> mnRaw
                else -> mnRaw.padStart(3, '0').takeLast(3)
            }
        return mc + mn
    }

    private fun validId(value: Int): Int? {
        if (value == Int.MAX_VALUE || value < 0) return null
        return value
    }

    private fun validLongId(value: Long): Long? {
        if (value == Long.MAX_VALUE || value < 0) return null
        return value
    }

    private fun signalDbm(strength: CellSignalStrength): Int {
        val dbm = strength.dbm
        return if (dbm != CellInfo.UNAVAILABLE) dbm else -113
    }

    /** WiGLE GSM ASU → dBm mapping. */
    private fun gsmAsuToDbm(asu: Int): Int =
        when (asu) {
            99 -> -113
            in 0..31 -> (asu * 2) - 113
            else -> -113
        }

    companion object {
        val CELL_SIGNAL_KINDS =
            setOf("GSM", "WCDMA", "LTE", "NR", "CDMA")

        fun isCellSignalKind(kind: String): Boolean =
            kind.uppercase() in CELL_SIGNAL_KINDS

        fun cellToJson(
            cell: CellObservation,
            observedUnixMs: Long,
            lat: Double,
            lon: Double,
            altitudeM: Int?,
            accuracyM: Float?,
        ): JSONObject =
            JSONObject().apply {
                put("cell_key", cell.cellKey)
                put("network_name", cell.networkName)
                put("capabilities", cell.capabilities)
                put("cell_type", cell.signalKind)
                put("frequency", cell.frequencyMhz)
                put("rssi", cell.rssi)
                put("observed_unix_ms", observedUnixMs)
                put("lat", lat)
                put("lon", lon)
                altitudeM?.let { put("altitude_m", it) }
                accuracyM?.let { put("accuracy_m", it) }
            }
    }
}
