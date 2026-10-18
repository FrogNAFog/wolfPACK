package net.wardriver.mobile.ui.map

import org.osmdroid.tileprovider.tilesource.OnlineTileSourceBase
import org.osmdroid.tileprovider.tilesource.TileSourceFactory
import org.osmdroid.tileprovider.tilesource.XYTileSource

object PackTileSources {
    private const val DARK_NAME = "PackCartoDark"
    private const val LIGHT_NAME = "PackOsmLight"

    private val cartoDark: OnlineTileSourceBase by lazy {
        XYTileSource(
            DARK_NAME,
            0,
            20,
            256,
            ".png",
            arrayOf(
                "https://a.basemaps.cartocdn.com/dark_all/",
                "https://b.basemaps.cartocdn.com/dark_all/",
                "https://c.basemaps.cartocdn.com/dark_all/",
                "https://d.basemaps.cartocdn.com/dark_all/",
            ),
            "© OpenStreetMap contributors © CARTO",
        )
    }

    private val osmLight: OnlineTileSourceBase by lazy {
        XYTileSource(
            LIGHT_NAME,
            0,
            19,
            256,
            ".png",
            arrayOf("https://tile.openstreetmap.org/"),
            "© OpenStreetMap contributors",
        )
    }

    fun tileSource(basemap: MapBasemap): OnlineTileSourceBase =
        when (basemap) {
            MapBasemap.Dark -> cartoDark
            MapBasemap.Light -> osmLight
        }

    fun attribution(basemap: MapBasemap): String =
        when (basemap) {
            MapBasemap.Dark -> "© OpenStreetMap © CARTO"
            MapBasemap.Light -> "© OpenStreetMap contributors"
        }

    /** Legacy MAPNIK cache name from builds before basemap preference. */
    val legacyMapnikName: String = TileSourceFactory.MAPNIK.name()
}
