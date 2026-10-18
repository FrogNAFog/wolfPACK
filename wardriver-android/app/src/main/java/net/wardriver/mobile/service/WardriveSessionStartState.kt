package net.wardriver.mobile.service

/** Wardrive foreground session lifecycle for dashboard Start/Stop. */
sealed class WardriveSessionStartState {
    data object Idle : WardriveSessionStartState()

    data object Starting : WardriveSessionStartState()

    data class Running(
        val sessionId: Long,
    ) : WardriveSessionStartState()

    data class Failed(
        val reason: String,
    ) : WardriveSessionStartState()

    val isRunning: Boolean
        get() = this is Running

    val canStart: Boolean
        get() = this is Idle || this is Failed
}
