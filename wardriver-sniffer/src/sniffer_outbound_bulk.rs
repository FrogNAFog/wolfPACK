//! Deferred wardrive bulk captured during off-control sweeps (budgeted heap store).

use wardriver_core::sniffer_frag::{
    count_ble_send_frags_with_extras, count_probe_send_frags, count_wifi_send_frags_with_extras,
};
use wardriver_core::sniffer_sweep::{
    ble_lane_charge_bytes, group_probe_rows, group_wifi_rows, lane_pressure, oldest_probe_batch,
    oldest_wifi_batch, probe_batch_keys, probe_row_charge_bytes, wifi_batch_keys,
    wifi_row_charge_bytes, OutboundWifiRow,
};
use wardriver_proto::{
    bulk_gps_matchable, ApEntryV1, BleEntryV1, ProbeReqBurstV1, ScanEntryExtras, BLE_ENTRY_SIZE,
};
use wardriver_shared::memory_policy::{
    OUTBOUND_BULK_BLE_BUDGET_BYTES, OUTBOUND_BULK_PROBE_BUDGET_BYTES,
    OUTBOUND_BULK_WIFI_BUDGET_BYTES,
};
use wardriver_shared::spiram_heap;
use wardriver_stats as stats;

/// Extended `join_before_dump` when pending bulk was not delivered within this wall interval.
pub const OUTBOUND_AGE_PRESSURE_MS: u64 = 60_000;

/// Which wardrive bulk lanes to join and dump after capture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BulkDumpPlan {
    Wifi,
    Ble,
}

/// Budgeted store for WiFi AP rows, probe bursts, and BLE observations awaiting dispatch.
pub struct OutboundBulkStore {
    wifi_budget: usize,
    probe_budget: usize,
    ble_budget: usize,
    wifi_rows: Vec<OutboundWifiRow>,
    probe_rows: Vec<(u32, u64, ProbeReqBurstV1)>,
    ble_scan_end_ms: u32,
    ble_scan_end_unix_ms: u64,
    ble_entries: Vec<BleEntryV1>,
    ble_extras: Vec<ScanEntryExtras>,
    /// Wall ms of last successful wardrive deliver ([`clear_on_delivered`]); wolfPACK age pressure.
    last_dump_delivered_ms: u64,
}

impl OutboundBulkStore {
    #[must_use]
    pub fn new_from_policy() -> Self {
        Self::new(
            OUTBOUND_BULK_WIFI_BUDGET_BYTES,
            OUTBOUND_BULK_PROBE_BUDGET_BYTES,
            OUTBOUND_BULK_BLE_BUDGET_BYTES,
        )
    }

    #[must_use]
    pub fn new(wifi_budget: usize, probe_budget: usize, ble_budget: usize) -> Self {
        Self {
            wifi_budget,
            probe_budget,
            ble_budget,
            wifi_rows: spiram_heap::vec_with_capacity(wifi_budget / wifi_row_charge_bytes().max(1)),
            probe_rows: spiram_heap::vec_with_capacity(
                probe_budget / probe_row_charge_bytes().max(1),
            ),
            ble_scan_end_ms: 0,
            ble_scan_end_unix_ms: 0,
            ble_entries: spiram_heap::vec_with_capacity(ble_budget / BLE_ENTRY_SIZE.max(1)),
            ble_extras: spiram_heap::vec_with_capacity(ble_budget / BLE_ENTRY_SIZE.max(1)),
            last_dump_delivered_ms: stats::now_ms(),
        }
    }

    #[must_use]
    pub fn wifi_bytes_used(&self) -> usize {
        self.wifi_rows.len() * wifi_row_charge_bytes()
    }

    #[must_use]
    pub fn probe_bytes_used(&self) -> usize {
        self.probe_rows.len() * probe_row_charge_bytes()
    }

    #[must_use]
    pub fn ble_bytes_used(&self) -> usize {
        ble_lane_charge_bytes(self.ble_entries.len())
    }

    /// Sum of accounted bytes across all lanes.
    pub fn bytes_used(&self) -> usize {
        self.wifi_bytes_used() + self.probe_bytes_used() + self.ble_bytes_used()
    }

    /// True when any lane is near capacity (fail-safe dump join path).
    pub fn is_bulk_pressure(&self) -> bool {
        lane_pressure(self.wifi_bytes_used(), self.wifi_budget)
            || lane_pressure(self.probe_bytes_used(), self.probe_budget)
            || lane_pressure(self.ble_bytes_used(), self.ble_budget)
    }

    /// True when no successful deliver for [`OUTBOUND_AGE_PRESSURE_MS`] (wolfPACK join fail-safe).
    pub fn is_age_pressure(&self) -> bool {
        stats::now_ms().saturating_sub(self.last_dump_delivered_ms) >= OUTBOUND_AGE_PRESSURE_MS
    }

    /// Extended `PeerJoin` retry window: store nearly full or deliver overdue.
    pub fn is_join_pressure(&self) -> bool {
        self.is_bulk_pressure() || self.is_age_pressure()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.wifi_rows.is_empty() && self.probe_rows.is_empty() && self.ble_entries.is_empty()
    }

    pub fn clear(&mut self) {
        self.wifi_rows.clear();
        self.probe_rows.clear();
        self.ble_scan_end_ms = 0;
        self.ble_scan_end_unix_ms = 0;
        self.ble_entries.clear();
        self.ble_extras.clear();
    }

    /// Merge per-dwell WiFi groups from a sweep hop; evict stale / over-budget rows.
    pub fn insert_wifi(
        &mut self,
        now_unix_ms: u64,
        groups: impl IntoIterator<Item = (u32, u64, Vec<ApEntryV1>, Vec<ScanEntryExtras>)>,
    ) {
        self.evict_wifi_until_fits();
        for (scan_end_ms, scan_end_unix_ms, entries, extras) in groups {
            let mut extras = extras;
            extras.resize(entries.len(), ScanEntryExtras::default());
            for (entry, extras) in entries.into_iter().zip(extras) {
                self.wifi_rows.push(OutboundWifiRow {
                    scan_end_ms,
                    scan_end_unix_ms,
                    entry,
                    extras,
                });
            }
        }
        self.evict_stale_gps(now_unix_ms);
        self.evict_wifi_until_fits();
    }

    /// Merge per-dwell probe groups from a sweep hop; evict stale / over-budget rows.
    pub fn insert_probes(
        &mut self,
        now_unix_ms: u64,
        groups: impl IntoIterator<Item = (u32, u64, Vec<ProbeReqBurstV1>)>,
    ) {
        self.evict_probe_until_fits();
        for (scan_end_ms, scan_end_unix_ms, bursts) in groups {
            for burst in bursts {
                self.probe_rows.push((scan_end_ms, scan_end_unix_ms, burst));
            }
        }
        self.evict_stale_gps(now_unix_ms);
        self.evict_probe_until_fits();
    }

    /// Merge one BLE sweep window; evict stale / over-budget rows.
    pub fn insert_ble(
        &mut self,
        now_unix_ms: u64,
        scan_end_ms: u32,
        scan_end_unix_ms: u64,
        entries: Vec<BleEntryV1>,
        mut extras: Vec<ScanEntryExtras>,
    ) {
        self.evict_ble_until_fits();
        extras.resize(entries.len(), ScanEntryExtras::default());
        if !entries.is_empty() && scan_end_ms >= self.ble_scan_end_ms {
            self.ble_scan_end_ms = scan_end_ms;
            self.ble_scan_end_unix_ms = scan_end_unix_ms;
        }
        self.ble_entries.extend(entries);
        self.ble_extras.extend(extras);
        self.evict_stale_gps(now_unix_ms);
        self.evict_ble_until_fits();
    }

    /// Group stored WiFi rows and probe tuples for ESP-NOW / HTTP dispatch.
    pub fn build_wifi_send_view(
        &self,
    ) -> (
        Vec<(u32, u64, Vec<ApEntryV1>, Vec<ScanEntryExtras>)>,
        Vec<(u32, u64, Vec<ProbeReqBurstV1>)>,
    ) {
        (
            group_wifi_rows(&self.wifi_rows),
            group_probe_rows(&self.probe_rows),
        )
    }

    /// Count encrypted send frags pending for the given dump lane (no clone of entry payloads).
    #[must_use]
    pub fn pending_send_frags(&self, plan: BulkDumpPlan) -> u32 {
        match plan {
            BulkDumpPlan::Wifi => {
                let (wifi_batches, probe_groups) = self.build_wifi_send_view();
                count_probe_send_frags(&probe_groups)
                    .saturating_add(count_wifi_send_frags_with_extras(&wifi_batches))
            }
            BulkDumpPlan::Ble => {
                let (_, _, ble_entries, ble_extras) = self.build_ble_send_view();
                count_ble_send_frags_with_extras(&ble_entries, &ble_extras)
            }
        }
    }

    /// BLE sweep anchor and entries for dispatch.
    #[must_use]
    pub fn build_ble_send_view(&self) -> (u32, u64, Vec<BleEntryV1>, Vec<ScanEntryExtras>) {
        (
            self.ble_scan_end_ms,
            self.ble_scan_end_unix_ms,
            self.ble_entries.clone(),
            self.ble_extras.clone(),
        )
    }

    /// Drop all pending bulk after a successful transport phase.
    pub fn clear_on_delivered(&mut self) {
        self.last_dump_delivered_ms = stats::now_ms();
        self.clear();
    }

    fn evict_stale_gps(&mut self, now_unix_ms: u64) {
        let wifi_batches = wifi_batch_keys(&self.wifi_rows);
        for (scan_end_ms, scan_end_unix_ms) in wifi_batches {
            if !bulk_gps_matchable(scan_end_unix_ms, now_unix_ms) {
                self.remove_wifi_batch(scan_end_ms, scan_end_unix_ms);
                stats::inc_sniffer_bulk_lost_cycle();
            }
        }

        let probe_batches = probe_batch_keys(&self.probe_rows);
        for (scan_end_ms, scan_end_unix_ms) in probe_batches {
            if !bulk_gps_matchable(scan_end_unix_ms, now_unix_ms) {
                self.remove_probe_batch(scan_end_ms, scan_end_unix_ms);
                stats::inc_sniffer_bulk_lost_cycle();
            }
        }

        if !self.ble_entries.is_empty()
            && !bulk_gps_matchable(self.ble_scan_end_unix_ms, now_unix_ms)
        {
            self.clear_ble_window();
            stats::inc_sniffer_bulk_lost_cycle();
        }
    }

    fn evict_wifi_until_fits(&mut self) {
        while self.wifi_bytes_used() > self.wifi_budget {
            let Some((scan_end_ms, scan_end_unix_ms)) = oldest_wifi_batch(&self.wifi_rows) else {
                break;
            };
            self.remove_wifi_batch(scan_end_ms, scan_end_unix_ms);
            stats::inc_sniffer_bulk_lost_cycle();
        }
    }

    fn evict_probe_until_fits(&mut self) {
        while self.probe_bytes_used() > self.probe_budget {
            let Some((scan_end_ms, scan_end_unix_ms)) = oldest_probe_batch(&self.probe_rows) else {
                break;
            };
            self.remove_probe_batch(scan_end_ms, scan_end_unix_ms);
            stats::inc_sniffer_bulk_lost_cycle();
        }
    }

    fn evict_ble_until_fits(&mut self) {
        while self.ble_bytes_used() > self.ble_budget {
            if self.ble_entries.is_empty() {
                break;
            }
            self.clear_ble_window();
            stats::inc_sniffer_bulk_lost_cycle();
        }
    }

    fn clear_ble_window(&mut self) {
        self.ble_scan_end_ms = 0;
        self.ble_scan_end_unix_ms = 0;
        self.ble_entries.clear();
        self.ble_extras.clear();
    }

    fn remove_wifi_batch(&mut self, scan_end_ms: u32, scan_end_unix_ms: u64) {
        self.wifi_rows.retain(|row| {
            row.scan_end_ms != scan_end_ms || row.scan_end_unix_ms != scan_end_unix_ms
        });
    }

    fn remove_probe_batch(&mut self, scan_end_ms: u32, scan_end_unix_ms: u64) {
        self.probe_rows
            .retain(|(ms, unix, _)| *ms != scan_end_ms || *unix != scan_end_unix_ms);
    }
}
