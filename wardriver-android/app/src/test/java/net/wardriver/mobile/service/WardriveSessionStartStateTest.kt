package net.wardriver.mobile.service

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class WardriveSessionStartStateTest {
    @Test
    fun idleCanStart() {
        assertTrue(WardriveSessionStartState.Idle.canStart)
        assertFalse(WardriveSessionStartState.Idle.isRunning)
    }

    @Test
    fun startingCannotStart() {
        assertFalse(WardriveSessionStartState.Starting.canStart)
        assertFalse(WardriveSessionStartState.Starting.isRunning)
    }

    @Test
    fun runningIsRunningNotCanStart() {
        val state = WardriveSessionStartState.Running(42L)
        assertTrue(state.isRunning)
        assertFalse(state.canStart)
    }

    @Test
    fun failedCanStartAgain() {
        val state = WardriveSessionStartState.Failed("blocked")
        assertTrue(state.canStart)
        assertFalse(state.isRunning)
    }
}
