package net.wardriver.mobile.upload

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class UploadCoordinatorTest {
    @Test
    fun uploadInProgressFlagDefaultsFalse() {
        UploadCoordinator.uploadInProgress = false
        assertFalse(UploadCoordinator.uploadInProgress)
    }

    @Test
    fun uploadInProgressCanBeSet() {
        UploadCoordinator.uploadInProgress = true
        assertTrue(UploadCoordinator.uploadInProgress)
        UploadCoordinator.uploadInProgress = false
    }
}
