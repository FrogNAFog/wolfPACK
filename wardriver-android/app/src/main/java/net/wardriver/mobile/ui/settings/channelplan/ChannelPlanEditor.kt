package net.wardriver.mobile.ui.settings.channelplan

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.FilterChip
import androidx.compose.material3.FilterChipDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import net.wardriver.mobile.data.AppPreferences
import net.wardriver.mobile.ui.components.PackOutlinedButton
import net.wardriver.mobile.ui.components.PackOutlinedTextField
import net.wardriver.mobile.ui.components.PackPrimaryButton
import net.wardriver.mobile.ui.components.PackSwitch
import net.wardriver.mobile.ui.theme.PackColor

/** Edit full control-parity channel plan; save pushes ChannelPlanSync / legacy PolicySync. */
@OptIn(ExperimentalLayoutApi::class)
@Composable
fun ChannelPlanEditor(
    plan: AppPreferences.ChannelPlan,
    channelPlanAckOk: Boolean?,
    channelPlanAckRevision: Long?,
    hopCycleMs: Long?,
    unassignedCount: Int?,
    supportsDedicatedSync: Boolean,
    onSave: (AppPreferences.ChannelPlan) -> Unit,
    onResetDefaults: () -> Unit,
    modifier: Modifier = Modifier,
) {
    var dwellText by rememberSaveable(plan.dwellMs) { mutableStateOf(plan.dwellMs.toString()) }
    var bandMode by rememberSaveable(plan.bandMode) { mutableStateOf(plan.bandMode) }
    var include24 by rememberSaveable(plan.include24Ghz) { mutableStateOf(plan.include24Ghz) }
    var include5 by rememberSaveable(plan.include5Ghz) { mutableStateOf(plan.include5Ghz) }
    var includeDfs by rememberSaveable(plan.includeDfs) { mutableStateOf(plan.includeDfs) }
    var selected24 by rememberSaveable(plan.channels24) { mutableStateOf(plan.channels24.toSet()) }
    var selected5 by rememberSaveable(plan.channels5) { mutableStateOf(plan.channels5.toSet()) }
    var selectedDfs by rememberSaveable(plan.channels5Dfs) { mutableStateOf(plan.channels5Dfs.toSet()) }
    var expand24 by rememberSaveable { mutableStateOf(true) }
    var expand5 by rememberSaveable { mutableStateOf(false) }
    var expandDfs by rememberSaveable { mutableStateOf(false) }
    var validationError by rememberSaveable { mutableStateOf<String?>(null) }

    val dwellPreview = dwellText.toIntOrNull() ?: plan.dwellMs
    val effectiveCount =
        (if (include24) selected24.size else 0) +
            (if (include5) selected5.size else 0) +
            (if (include5 && includeDfs) selectedDfs.size else 0)
    val hopEstimateMs = effectiveCount * dwellPreview.coerceAtLeast(1)

    Column(
        modifier = modifier.fillMaxWidth().padding(vertical = 8.dp),
        verticalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        Text(
            text =
                if (supportsDedicatedSync) {
                    "Dedicated ChannelPlanSync (rev ${plan.revision})"
                } else {
                    "Legacy PolicySync flat channel list"
                },
            style = MaterialTheme.typography.bodySmall,
            color = PackColor.Muted,
        )
        PackOutlinedTextField(
            value = dwellText,
            onValueChange = {
                dwellText = it.filter { ch -> ch.isDigit() }
                validationError = null
            },
            label = { Text("Dwell (ms)") },
            modifier = Modifier.fillMaxWidth(),
            singleLine = true,
        )
        Row(
            modifier = Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            FilterChip(
                selected = bandMode == AppPreferences.BAND_MODE_AUTO,
                onClick = { bandMode = AppPreferences.BAND_MODE_AUTO },
                label = { Text("Auto balance") },
                modifier = Modifier.weight(1f),
                colors = packChipColors(),
            )
            FilterChip(
                selected = bandMode == AppPreferences.BAND_MODE_MANUAL,
                onClick = { bandMode = AppPreferences.BAND_MODE_MANUAL },
                label = { Text("Manual bands") },
                modifier = Modifier.weight(1f),
                colors = packChipColors(),
            )
        }
        if (bandMode == AppPreferences.BAND_MODE_MANUAL) {
            Text(
                text = "Assign 2.4 / 5 GHz per WiFi sniffer on the Sniffers tab.",
                style = MaterialTheme.typography.bodySmall,
                color = PackColor.Muted,
            )
        }

        BandPoolCard(
            title = "2.4 GHz",
            expanded = expand24,
            onToggleExpand = { expand24 = !expand24 },
            enabled = include24,
            onEnabledChange = { include24 = it },
            options = CHANNELS_24_OPTIONS,
            selected = selected24,
            onSelectedChange = { selected24 = it },
            presets =
                listOf(
                    "All" to AppPreferences.DEFAULT_CHANNELS_24.toSet(),
                    "Legacy" to setOf(1, 6, 11),
                ),
        )
        BandPoolCard(
            title = "5 GHz (non-DFS)",
            expanded = expand5,
            onToggleExpand = { expand5 = !expand5 },
            enabled = include5,
            onEnabledChange = { include5 = it },
            options = CHANNELS_5_OPTIONS,
            selected = selected5,
            onSelectedChange = { selected5 = it },
            presets =
                listOf(
                    "UNII-1" to setOf(36, 40, 44, 48),
                    "UNII-3" to setOf(149, 153, 157, 161),
                    "All" to AppPreferences.DEFAULT_CHANNELS_5.toSet(),
                ),
        )
        BandPoolCard(
            title = "5 GHz DFS",
            expanded = expandDfs,
            onToggleExpand = { expandDfs = !expandDfs },
            enabled = includeDfs,
            onEnabledChange = { includeDfs = it },
            enabledVisible = include5,
            options = CHANNELS_5_DFS_OPTIONS,
            selected = selectedDfs,
            onSelectedChange = { selectedDfs = it },
            presets = listOf("All DFS" to AppPreferences.DEFAULT_CHANNELS_5_DFS.toSet()),
        )

        Text(
            text = buildEffectiveSummary(effectiveCount, hopEstimateMs, channelPlanAckOk, channelPlanAckRevision, hopCycleMs, unassignedCount),
            style = MaterialTheme.typography.bodySmall,
            color = PackColor.Muted,
        )
        validationError?.let { err ->
            Text(text = err, style = MaterialTheme.typography.bodySmall, color = PackColor.Err)
        }
        PackPrimaryButton(
            text = "Save plan",
            onClick = {
                val dwell = dwellText.toIntOrNull()
                when {
                    dwell == null || dwell !in AppPreferences.MIN_DWELL_MS..AppPreferences.MAX_DWELL_MS ->
                        validationError = "Dwell must be ${AppPreferences.MIN_DWELL_MS}-${AppPreferences.MAX_DWELL_MS} ms"
                    !include24 && !include5 ->
                        validationError = "Enable at least one band"
                    include24 && selected24.isEmpty() ->
                        validationError = "Select at least one 2.4 GHz channel"
                    include5 && selected5.isEmpty() && !(includeDfs && selectedDfs.isNotEmpty()) ->
                        validationError = "Select 5 GHz or DFS channels"
                    include5 && includeDfs && selectedDfs.isEmpty() && selected5.isEmpty() ->
                        validationError = "Select 5 GHz or DFS channels"
                    else -> {
                        validationError = null
                        onSave(
                            AppPreferences.ChannelPlan(
                                revision = plan.revision,
                                dwellMs = dwell,
                                include24Ghz = include24,
                                include5Ghz = include5,
                                includeDfs = includeDfs,
                                bandMode = bandMode,
                                channels24 = selected24.sorted(),
                                channels5 = selected5.sorted(),
                                channels5Dfs = selectedDfs.sorted(),
                            ),
                        )
                    }
                }
            },
            modifier = Modifier.fillMaxWidth(),
        )
        PackOutlinedButton(
            text = "Reset defaults",
            onClick = onResetDefaults,
            modifier = Modifier.fillMaxWidth(),
        )
    }
}

@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun BandPoolCard(
    title: String,
    expanded: Boolean,
    onToggleExpand: () -> Unit,
    enabled: Boolean,
    onEnabledChange: (Boolean) -> Unit,
    options: List<Int>,
    selected: Set<Int>,
    onSelectedChange: (Set<Int>) -> Unit,
    presets: List<Pair<String, Set<Int>>>,
    enabledVisible: Boolean = true,
) {
    Column(
        modifier =
            Modifier
                .fillMaxWidth()
                .padding(vertical = 4.dp),
    ) {
        Row(
            modifier =
                Modifier
                    .fillMaxWidth()
                    .clickable(onClick = onToggleExpand)
                    .padding(vertical = 4.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.SpaceBetween,
        ) {
            Text(text = title, style = MaterialTheme.typography.titleSmall, color = PackColor.Text)
            Row(verticalAlignment = Alignment.CenterVertically) {
                if (enabledVisible) {
                    PackSwitch(
                        checked = enabled,
                        onCheckedChange = onEnabledChange,
                    )
                }
                Text(
                    text = if (expanded) "-" else "+",
                    color = PackColor.Muted,
                    modifier = Modifier.padding(start = 8.dp),
                )
            }
        }
        if (expanded && enabled && enabledVisible) {
            FlowRow(
                horizontalArrangement = Arrangement.spacedBy(6.dp),
                verticalArrangement = Arrangement.spacedBy(4.dp),
                modifier = Modifier.padding(bottom = 6.dp),
            ) {
                presets.forEach { (label, preset) ->
                    FilterChip(
                        selected = selected == preset,
                        onClick = { onSelectedChange(preset) },
                        label = { Text(label) },
                        colors = packChipColors(),
                    )
                }
            }
            FlowRow(
                horizontalArrangement = Arrangement.spacedBy(6.dp),
                verticalArrangement = Arrangement.spacedBy(4.dp),
            ) {
                options.forEach { ch ->
                    FilterChip(
                        selected = ch in selected,
                        onClick = {
                            onSelectedChange(
                                if (ch in selected) selected - ch else selected + ch,
                            )
                        },
                        label = { Text(ch.toString()) },
                        colors = packChipColors(),
                    )
                }
            }
        }
    }
}

@Composable
private fun packChipColors() =
    FilterChipDefaults.filterChipColors(
        containerColor = PackColor.Surface2,
        labelColor = PackColor.Text,
        selectedContainerColor = PackColor.Cyan,
        selectedLabelColor = PackColor.Background,
    )

private fun buildEffectiveSummary(
    channelCount: Int,
    hopEstimateMs: Int,
    ackOk: Boolean?,
    ackRevision: Long?,
    hopCycleMs: Long?,
    unassignedCount: Int?,
): String {
    val parts = mutableListOf<String>()
    parts.add("$channelCount channels")
    parts.add("~${hopEstimateMs} ms hop cycle (single sniffer)")
    hopCycleMs?.takeIf { it > 0L }?.let { parts.add("gateway hop ${it} ms") }
    unassignedCount?.takeIf { it > 0 }?.let { parts.add("$it unassigned") }
    when (ackOk) {
        true -> parts.add("applied rev ${ackRevision ?: "?"}")
        false -> parts.add("gateway rejected plan")
        null -> {}
    }
    return parts.joinToString(" - ")
}

private val CHANNELS_24_OPTIONS = (1..14).toList()
private val CHANNELS_5_OPTIONS =
    listOf(36, 40, 44, 48, 149, 153, 157, 161, 165, 169, 173, 177)
private val CHANNELS_5_DFS_OPTIONS =
    listOf(52, 56, 60, 64, 100, 104, 108, 112, 116, 120, 124, 128, 132, 136, 140, 144)
