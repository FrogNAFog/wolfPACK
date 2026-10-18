package net.wardriver.mobile.notify

import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.SharedFlow
import kotlinx.coroutines.flow.asSharedFlow

/** Ephemeral in-app alert messages (banners while app is foreground). */
object AlertBus {
    /** Alert family carried on the shared bus. */
    enum class Kind {
        /** Camera / Remote ID detection alerts (snackbar-style banner). */
        Detection,

        /** Wardrive geofence enter/leave (top banner). */
        GeofenceEnter,

        GeofenceLeave,
    }

    data class Message(
        val kind: Kind,
        val title: String,
        val detail: String,
        val suppressedCount: Int,
    )

    private val _events = MutableSharedFlow<Message>(extraBufferCapacity = 8)
    val events: SharedFlow<Message> = _events.asSharedFlow()

    fun emit(
        kind: Kind,
        title: String,
        detail: String,
        suppressedCount: Int = 0,
    ) {
        _events.tryEmit(Message(kind, title, detail, suppressedCount))
    }
}
