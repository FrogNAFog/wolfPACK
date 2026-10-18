package net.wardriver.mobile.data

/**
 * Outcome of assembling and enqueueing PackLink policy (and optional channel plan) to the gateway.
 *
 * [Pushed] means the TCP writer flushed the frame(s); [PolicySyncAck] is a separate application ack.
 */
sealed class PolicyPushResult {
    data object NotConnected : PolicyPushResult()

    /** JNI pack returned null (invalid roster JSON, including negative wire u32 IDs). */
    data object PackFailed : PolicyPushResult()

    /** Queued frames were superseded by a newer push before write. */
    data object Superseded : PolicyPushResult()

    /** Session generation changed before the writer flushed the frame. */
    data object StaleSession : PolicyPushResult()

    /** TCP write failed on the PackLink socket. */
    data class WriteFailed(val message: String) : PolicyPushResult()

    /** Frame(s) written to the gateway TCP session (ack may still be pending). */
    data object Pushed : PolicyPushResult()

    val succeeded: Boolean
        get() = this == Pushed
}
