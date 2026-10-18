//! Roster gate host tests (phone-owned PolicySyncV1 roster).

use wardriver_fleet_realtime::{roster_allows, roster_fleet_slot, PolicySyncV1, RosterEntryV1};
use wardriver_packlink::ROSTER_MAX;

fn sample_policy(rostered: bool) -> PolicySyncV1 {
    let mac = [0xDE, 0xAD, 0xBE, 0xEF, 0x00, 0x01];
    PolicySyncV1 {
        scan_allowed: true,
        wardrive_suppressed: false,
        geofence_enabled: false,
        geofence_lat_min: 0.0,
        geofence_lat_max: 0.0,
        geofence_lon_min: 0.0,
        geofence_lon_max: 0.0,
        denylist_bssids: Vec::new(),
        denylist_ssids: Vec::new(),
        roster: if rostered {
            vec![RosterEntryV1 {
                sniffer_id: 42,
                mac,
                fleet_slot: 3,
                role: 0,
            }]
        } else {
            Vec::new()
        },
        dwell_ms: 0,
        chan_count: 0,
        channels: [0u8; wardriver_proto::CHANNELS_MAX],
        detection_disable_wifi_mask: 0,
        detection_disable_ble_mask: 0,
        ssid_watch: Vec::new(),
        remote_id_disable_mask: 0,
        ble_active_scan: false,
    }
}

#[test]
fn non_rostered_sniffer_rejected() {
    let policy = sample_policy(false);
    let mac = [0xDE, 0xAD, 0xBE, 0xEF, 0x00, 0x01];
    assert!(!roster_allows(&policy, 42, &mac));
    assert!(!roster_allows(&policy, 99, &mac));
}

#[test]
fn rostered_pair_allowed() {
    let policy = sample_policy(true);
    let mac = [0xDE, 0xAD, 0xBE, 0xEF, 0x00, 0x01];
    assert!(roster_allows(&policy, 42, &mac));
    // Wrong MAC for same sniffer_id must fail (binding invariant).
    assert!(!roster_allows(
        &policy,
        42,
        &[0x00, 0x11, 0x22, 0x33, 0x44, 0x55]
    ));
    // Wrong sniffer_id with correct MAC must fail.
    assert!(!roster_allows(&policy, 7, &mac));
}

#[test]
fn roster_fleet_slot_returns_dashboard_slot() {
    let policy = sample_policy(true);
    // Slot is the stable dashboard index for adopted sniffers (1..=N).
    assert_eq!(roster_fleet_slot(&policy, 42), Some(3));
    assert_eq!(roster_fleet_slot(&policy, 99), None);
}

#[test]
fn multiple_roster_entries_independent_binding() {
    let mac_a = [0xDE, 0xAD, 0xBE, 0xEF, 0x00, 0x01];
    let mac_b = [0xDE, 0xAD, 0xBE, 0xEF, 0x00, 0x02];
    let policy = PolicySyncV1 {
        scan_allowed: true,
        wardrive_suppressed: false,
        geofence_enabled: false,
        geofence_lat_min: 0.0,
        geofence_lat_max: 0.0,
        geofence_lon_min: 0.0,
        geofence_lon_max: 0.0,
        denylist_bssids: Vec::new(),
        denylist_ssids: Vec::new(),
        roster: vec![
            RosterEntryV1 {
                sniffer_id: 10,
                mac: mac_a,
                fleet_slot: 1,
                role: 0,
            },
            RosterEntryV1 {
                sniffer_id: 20,
                mac: mac_b,
                fleet_slot: 2,
                role: 1,
            },
        ],
        dwell_ms: 0,
        chan_count: 0,
        channels: [0u8; wardriver_proto::CHANNELS_MAX],
        detection_disable_wifi_mask: 0,
        detection_disable_ble_mask: 0,
        ssid_watch: Vec::new(),
        remote_id_disable_mask: 0,
        ble_active_scan: false,
    };
    // Each (sniffer_id, mac) pair is allowed only when both fields match the roster entry.
    assert!(roster_allows(&policy, 10, &mac_a));
    assert!(roster_allows(&policy, 20, &mac_b));
    assert!(!roster_allows(&policy, 10, &mac_b));
    assert!(!roster_allows(&policy, 20, &mac_a));
    assert_eq!(roster_fleet_slot(&policy, 10), Some(1));
    assert_eq!(roster_fleet_slot(&policy, 20), Some(2));
}

#[test]
fn roster_max_capacity_last_entry_still_gated() {
    // PolicySyncV1 supports up to ROSTER_MAX (48) sniffers; last slot must bind like the first.
    let mut roster = Vec::with_capacity(ROSTER_MAX);
    for i in 0..ROSTER_MAX {
        let id = (i + 1) as u32;
        roster.push(RosterEntryV1 {
            sniffer_id: id,
            mac: [0x02, 0x00, 0x00, 0x00, 0x00, i as u8],
            fleet_slot: (i + 1) as u8,
            role: 0,
        });
    }
    let policy = PolicySyncV1 {
        scan_allowed: true,
        wardrive_suppressed: false,
        geofence_enabled: false,
        geofence_lat_min: 0.0,
        geofence_lat_max: 0.0,
        geofence_lon_min: 0.0,
        geofence_lon_max: 0.0,
        denylist_bssids: Vec::new(),
        denylist_ssids: Vec::new(),
        roster,
        dwell_ms: 0,
        chan_count: 0,
        channels: [0u8; wardriver_proto::CHANNELS_MAX],
        detection_disable_wifi_mask: 0,
        detection_disable_ble_mask: 0,
        ssid_watch: Vec::new(),
        remote_id_disable_mask: 0,
        ble_active_scan: false,
    };
    let last_id = ROSTER_MAX as u32;
    let last_mac = [0x02, 0x00, 0x00, 0x00, 0x00, (ROSTER_MAX - 1) as u8];
    assert!(roster_allows(&policy, last_id, &last_mac));
    assert_eq!(roster_fleet_slot(&policy, last_id), Some(ROSTER_MAX as u8));
    // Wrong MAC for last sniffer_id is rejected.
    assert!(!roster_allows(&policy, last_id, &[0xFF; 6]));
}
