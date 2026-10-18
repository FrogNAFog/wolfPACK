package net.wardriver.mobile.ui.map

import net.wardriver.mobile.ui.theme.PackColor
import org.junit.Assert.assertEquals
import org.junit.Test

class MapMarkerFactoryTest {
    @Test
    fun wifiApUsesMapWifiColor() {
        assertEquals(PackColor.argb(PackColor.MapWifi), MapMarkerFactory.apFillArgb("WIFI"))
        assertEquals(PackColor.argb(PackColor.MapWifi), MapMarkerFactory.apFillArgb("wifi"))
    }

    @Test
    fun bleApUsesMapBleColor() {
        assertEquals(PackColor.argb(PackColor.MapBle), MapMarkerFactory.apFillArgb("BLE"))
    }

    @Test
    fun unknownApKindDefaultsToWifi() {
        assertEquals(PackColor.argb(PackColor.MapWifi), MapMarkerFactory.apFillArgb("LTE"))
    }

    @Test
    fun alertUsesMapAlertColor() {
        assertEquals(PackColor.argb(PackColor.MapAlert), MapMarkerFactory.alertFillArgb())
    }
}
