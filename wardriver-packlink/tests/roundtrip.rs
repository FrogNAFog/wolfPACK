//! PackLink v1 round-trip host tests.

use wardriver_packlink::{
    adopt_progress, adopt_reason, capture_error, capture_job_id, default_gateway_info, event_kind,
    flags, pack_capture_cmd, pack_capture_status, pack_event_notify, pack_frame,
    pack_observation_bulk, pack_policy_sync_seq, pack_sniffer_status, pack_sniffer_status_batch,
    pack_spool_replay, pack_time_sync, policy_has_fleet_config, policy_sync_payload_bytes,
    roster_allows, unpack_capture_cmd, unpack_capture_status, unpack_event_notify, unpack_frame,
    unpack_observation_bulk, unpack_policy_sync, unpack_sniffer_status,
    unpack_sniffer_status_batch, unpack_spool_replay, unpack_time_sync, CaptureCmdV1,
    CaptureStatusV1, EventNotifyV1, MsgType, ObservationBulkV1, PolicySyncV1, RosterEntryV1,
    SnifferStatusV1, SpoolReplayV1, TimeSyncV1, PACKLINK_VERSION, ROSTER_MAX,
};
use wardriver_proto::CHANNELS_MAX;

#[test]
fn frame_crc_roundtrip() {
    let payload = b"hello-packlink";
    let wire = pack_frame(MsgType::GatewayInfo, 0, 42, payload);
    let frame = unpack_frame(&wire).expect("unpack");
    assert_eq!(frame.version, PACKLINK_VERSION);
    assert_eq!(frame.msg_type, MsgType::GatewayInfo);
    assert_eq!(frame.seq, 42);
    assert_eq!(frame.payload, payload);
}

#[test]
fn frame_rejects_bad_crc() {
    let mut wire = pack_frame(MsgType::TimeSync, 0, 1, &[0u8; 4]);
    let last = wire.len() - 1;
    wire[last] ^= 0xFF;
    assert!(unpack_frame(&wire).is_err());
}

#[test]
fn policy_sync_roundtrip() {
    let mut channels = [0u8; CHANNELS_MAX];
    channels[0] = 1;
    channels[1] = 6;
    channels[2] = 11;
    let policy = PolicySyncV1 {
        scan_allowed: true,
        wardrive_suppressed: false,
        geofence_enabled: true,
        geofence_lat_min: 37.77,
        geofence_lat_max: 37.78,
        geofence_lon_min: -122.42,
        geofence_lon_max: -122.41,
        denylist_bssids: vec![[0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF]],
        denylist_ssids: vec!["HiddenNet".to_string()],
        roster: vec![
            RosterEntryV1 {
                sniffer_id: 1,
                mac: [0x02, 0x11, 0x22, 0x33, 0x44, 0x55],
                fleet_slot: 0,
                role: 0,
            },
            RosterEntryV1 {
                sniffer_id: 2,
                mac: [0x02, 0xAA, 0xBB, 0xCC, 0xDD, 0xEE],
                fleet_slot: 1,
                role: 1,
            },
        ],
        dwell_ms: 250,
        chan_count: 3,
        channels,
        detection_disable_wifi_mask: 0x0000_0002,
        detection_disable_ble_mask: 0x0000_0008,
        ssid_watch: vec!["WatchMe".to_string()],
        remote_id_disable_mask: 0x0000_0004,
        ble_active_scan: false,
    };
    let wire = pack_policy_sync_seq(&policy, 0);
    let frame = unpack_frame(&wire).expect("frame");
    let back = unpack_policy_sync(frame.payload).expect("policy");
    assert_eq!(back, policy);
}

#[test]
fn policy_sync_payload_bytes_matches_frame_inner() {
    let mut channels = [0u8; CHANNELS_MAX];
    channels[0] = 1;
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
        roster: Vec::new(),
        dwell_ms: 200,
        chan_count: 1,
        channels,
        detection_disable_wifi_mask: 0,
        detection_disable_ble_mask: 0,
        ssid_watch: Vec::new(),
        remote_id_disable_mask: 0,
        ble_active_scan: false,
    };
    let wire = pack_policy_sync_seq(&policy, 0);
    let frame = unpack_frame(&wire).expect("frame");
    let inner = policy_sync_payload_bytes(&policy);
    assert_eq!(frame.payload, inner.as_slice());
    let back = unpack_policy_sync(&inner).expect("inner unpack");
    assert_eq!(back.dwell_ms, policy.dwell_ms);
}

#[test]
fn policy_sync_roster_48_roundtrip() {
    let mut roster = Vec::with_capacity(ROSTER_MAX);
    for i in 0..ROSTER_MAX {
        roster.push(RosterEntryV1 {
            sniffer_id: i as u32 + 1,
            mac: [
                0x02,
                (i as u8).wrapping_add(1),
                (i as u8).wrapping_add(2),
                (i as u8).wrapping_add(3),
                (i as u8).wrapping_add(4),
                (i as u8).wrapping_add(5),
            ],
            fleet_slot: i as u8,
            role: (i % 2) as u8,
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
        dwell_ms: 100,
        chan_count: 0,
        channels: [0u8; CHANNELS_MAX],
        detection_disable_wifi_mask: 0,
        detection_disable_ble_mask: 0,
        ssid_watch: Vec::new(),
        remote_id_disable_mask: 0,
        ble_active_scan: false,
    };
    let wire = pack_policy_sync_seq(&policy, 0);
    let frame = unpack_frame(&wire).expect("frame");
    let back = unpack_policy_sync(frame.payload).expect("policy");
    assert_eq!(back.roster.len(), ROSTER_MAX);
    assert_eq!(back, policy);
}

#[test]
fn policy_sync_roster_role_roundtrip() {
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
        roster: vec![RosterEntryV1 {
            sniffer_id: 11,
            mac: [0x02, 0x11, 0x22, 0x33, 0x44, 0x55],
            fleet_slot: 2,
            role: 1,
        }],
        dwell_ms: 0,
        chan_count: 0,
        channels: [0u8; CHANNELS_MAX],
        detection_disable_wifi_mask: 0,
        detection_disable_ble_mask: 0,
        ssid_watch: Vec::new(),
        remote_id_disable_mask: 0,
        ble_active_scan: false,
    };
    let wire = pack_policy_sync_seq(&policy, 0);
    let frame = unpack_frame(&wire).expect("frame");
    let back = unpack_policy_sync(frame.payload).expect("policy");
    assert_eq!(back.roster[0].role, 1);
    assert_eq!(back, policy);
}

#[test]
fn policy_sync_ble_active_scan_roundtrip() {
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
        roster: Vec::new(),
        dwell_ms: 0,
        chan_count: 0,
        channels: [0u8; CHANNELS_MAX],
        detection_disable_wifi_mask: 0,
        detection_disable_ble_mask: 0,
        ssid_watch: Vec::new(),
        remote_id_disable_mask: 0,
        ble_active_scan: true,
    };
    let wire = pack_policy_sync_seq(&policy, 0);
    let frame = unpack_frame(&wire).expect("frame");
    let back = unpack_policy_sync(frame.payload).expect("policy");
    assert!(back.ble_active_scan);
    assert_eq!(back, policy);
}

#[test]
fn policy_sync_legacy_decode_without_ble_active_scan() {
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
        roster: Vec::new(),
        dwell_ms: 250,
        chan_count: 0,
        channels: [0u8; CHANNELS_MAX],
        detection_disable_wifi_mask: 0,
        detection_disable_ble_mask: 0,
        ssid_watch: Vec::new(),
        remote_id_disable_mask: 0x0000_0010,
        ble_active_scan: true,
    };
    let payload = policy_sync_payload_bytes(&policy);
    // Drop the trailing ble_active_scan byte to simulate a pre-ble-active payload.
    let legacy_len = payload.len() - 1;
    let back = unpack_policy_sync(&payload[..legacy_len]).expect("legacy policy");
    assert_eq!(back.remote_id_disable_mask, 0x0000_0010);
    assert!(!back.ble_active_scan);
}

#[test]
fn roster_allows_adopted_pair() {
    let mac = [0x02, 0x11, 0x22, 0x33, 0x44, 0x55];
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
        roster: vec![RosterEntryV1 {
            sniffer_id: 7,
            mac,
            fleet_slot: 3,
            role: 0,
        }],
        dwell_ms: 0,
        chan_count: 0,
        channels: [0u8; CHANNELS_MAX],
        detection_disable_wifi_mask: 0,
        detection_disable_ble_mask: 0,
        ssid_watch: Vec::new(),
        remote_id_disable_mask: 0,
        ble_active_scan: false,
    };
    assert!(roster_allows(&policy, 7, &mac));
    assert!(!roster_allows(&policy, 7, &[0xFF; 6]));
    assert!(!roster_allows(&policy, 8, &mac));
}

#[test]
fn policy_has_fleet_config_roster_or_scan() {
    let empty = PolicySyncV1 {
        scan_allowed: false,
        wardrive_suppressed: false,
        geofence_enabled: false,
        geofence_lat_min: 0.0,
        geofence_lat_max: 0.0,
        geofence_lon_min: 0.0,
        geofence_lon_max: 0.0,
        denylist_bssids: Vec::new(),
        denylist_ssids: Vec::new(),
        roster: Vec::new(),
        dwell_ms: 0,
        chan_count: 0,
        channels: [0u8; CHANNELS_MAX],
        detection_disable_wifi_mask: 0,
        detection_disable_ble_mask: 0,
        ssid_watch: Vec::new(),
        remote_id_disable_mask: 0,
        ble_active_scan: false,
    };
    assert!(!policy_has_fleet_config(&empty));

    let scan_only = PolicySyncV1 {
        scan_allowed: true,
        ..empty.clone()
    };
    assert!(policy_has_fleet_config(&scan_only));

    let roster_only = PolicySyncV1 {
        roster: vec![RosterEntryV1 {
            sniffer_id: 1,
            mac: [0x02; 6],
            fleet_slot: 1,
            role: 0,
        }],
        ..empty
    };
    assert!(policy_has_fleet_config(&roster_only));
}

#[test]
fn observation_bulk_roundtrip() {
    let bulk = ObservationBulkV1 {
        batch_id: 99,
        sniffer_id: 3,
        scan_end_unix_ms: 1_700_000_000_000,
        wifi_ap_count: 2,
        ble_count: 1,
        payload: vec![1, 2, 3, 4],
    };
    let wire = pack_observation_bulk(&bulk, 7, flags::FRAGMENT_END);
    let frame = unpack_frame(&wire).expect("frame");
    assert_eq!(frame.flags & flags::FRAGMENT_END, flags::FRAGMENT_END);
    let back = unpack_observation_bulk(frame.payload).expect("bulk");
    assert_eq!(back, bulk);
}

#[test]
fn event_notify_priority_flag() {
    let ev = EventNotifyV1 {
        event_kind: event_kind::DETECTION_WIFI,
        sniffer_id: 1,
        unix_ms: 1_700_000_000_000,
        payload_len: 2,
        payload: vec![0xAB, 0xCD],
    };
    let wire = pack_event_notify(&ev, 1);
    let frame = unpack_frame(&wire).expect("frame");
    assert_ne!(frame.flags & flags::PRIORITY_ALERT, 0);
    let back = unpack_event_notify(frame.payload).expect("event");
    assert_eq!(back, ev);
}

#[test]
fn event_notify_remote_id_priority_flag() {
    let ev = EventNotifyV1 {
        event_kind: event_kind::REMOTE_ID,
        sniffer_id: 1,
        unix_ms: 1_700_000_000_000,
        payload_len: 2,
        payload: vec![0xAB, 0xCD],
    };
    let wire = pack_event_notify(&ev, 2);
    let frame = unpack_frame(&wire).expect("frame");
    assert_ne!(frame.flags & flags::PRIORITY_ALERT, 0);
}

#[test]
fn policy_sync_legacy_without_remote_id_mask() {
    let mut channels = [0u8; CHANNELS_MAX];
    channels[0] = 6;
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
        roster: Vec::new(),
        dwell_ms: 250,
        chan_count: 1,
        channels,
        detection_disable_wifi_mask: 0,
        detection_disable_ble_mask: 0,
        ssid_watch: vec!["WatchMe".to_string()],
        remote_id_disable_mask: 0,
        ble_active_scan: false,
    };
    let payload = policy_sync_payload_bytes(&policy);
    // Truncate remote_id_disable_mask(4) + ble_active_scan(1) to simulate pre-remote-id payload.
    let legacy_len = payload.len() - 5;
    let back = unpack_policy_sync(&payload[..legacy_len]).expect("legacy policy");
    assert_eq!(back.remote_id_disable_mask, 0);
    assert!(!back.ble_active_scan);
}

#[test]
fn spool_replay_roundtrip() {
    let chunk = SpoolReplayV1 {
        spool_offset: 4096,
        chunk_total: 10,
        chunk_index: 3,
        payload: vec![9; 128],
    };
    let wire = pack_spool_replay(&chunk, 5);
    let frame = unpack_frame(&wire).expect("frame");
    assert_eq!(frame.msg_type, MsgType::SpoolReplay);
    let back = unpack_spool_replay(frame.payload).expect("spool");
    assert_eq!(back, chunk);
}

#[test]
fn time_sync_roundtrip() {
    let ts = TimeSyncV1 {
        phone_unix_ms: 1_700_000_000_123,
        phone_mono_ms: 50_000,
        gateway_mono_ms: 48_000,
        time_set_seq: 12,
    };
    let wire = pack_time_sync(&ts, 2);
    let frame = unpack_frame(&wire).expect("frame");
    let back = unpack_time_sync(frame.payload).expect("time");
    assert_eq!(back, ts);
}

#[test]
fn policy_sync_legacy_decode_without_detection_tail() {
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
        roster: Vec::new(),
        dwell_ms: 100,
        chan_count: 1,
        channels: {
            let mut c = [0u8; CHANNELS_MAX];
            c[0] = 6;
            c
        },
        detection_disable_wifi_mask: 0,
        detection_disable_ble_mask: 0,
        ssid_watch: Vec::new(),
        remote_id_disable_mask: 0,
        ble_active_scan: false,
    };
    let wire = pack_policy_sync_seq(&policy, 0);
    let frame = unpack_frame(&wire).expect("frame");
    // Simulate pre-detection phone: payload ends after channel schedule (no flock/watch tail).
    let legacy_len = frame.payload.len() - 9;
    let back = unpack_policy_sync(&frame.payload[..legacy_len]).expect("legacy policy");
    assert_eq!(back.dwell_ms, 100);
    assert_eq!(back.chan_count, 1);
    assert_eq!(back.detection_disable_wifi_mask, 0);
    assert_eq!(back.detection_disable_ble_mask, 0);
    assert!(back.ssid_watch.is_empty());
}

#[test]
fn sniffer_status_single_roundtrip() {
    let row = SnifferStatusV1 {
        sniffer_id: 7,
        mac: [0x02, 0x11, 0x22, 0x33, 0x44, 0x55],
        last_seen_ms: 1_234_567,
        state: 2,
        role: 0,
        cpu_load_pct: 42,
        heap_avail_pct: 88,
        fleet_slot: 3,
        reboot_count: 1,
        adoption_state: 1,
        plan_ext: None,
        adopt_ext: None,
    };
    let wire = pack_sniffer_status(&row, 99_000, 11);
    let frame = unpack_frame(&wire).expect("frame");
    assert_eq!(frame.msg_type, MsgType::SnifferStatus);
    let (gateway_mono_ms, back) = unpack_sniffer_status(frame.payload).expect("row");
    assert_eq!(gateway_mono_ms, 99_000);
    assert_eq!(back, row);
}

#[test]
fn sniffer_status_batch_roundtrip() {
    let rows = vec![
        SnifferStatusV1 {
            sniffer_id: 1,
            mac: [0x02, 0xAA, 0xBB, 0xCC, 0xDD, 0x01],
            last_seen_ms: 100,
            state: 1,
            role: 0,
            cpu_load_pct: 10,
            heap_avail_pct: 90,
            fleet_slot: 1,
            reboot_count: 0,
            adoption_state: 1,
            plan_ext: None,
            adopt_ext: None,
        },
        SnifferStatusV1 {
            sniffer_id: 2,
            mac: [0x02, 0xAA, 0xBB, 0xCC, 0xDD, 0x02],
            last_seen_ms: 200,
            state: 3,
            role: 1,
            cpu_load_pct: 55,
            heap_avail_pct: 70,
            fleet_slot: 2,
            reboot_count: 2,
            adoption_state: 2,
            plan_ext: None,
            adopt_ext: None,
        },
    ];
    let wire = pack_sniffer_status_batch(&rows, 50_000, 3);
    let frame = unpack_frame(&wire).expect("frame");
    let (gateway_mono_ms, back) = unpack_sniffer_status_batch(frame.payload).expect("batch");
    assert_eq!(gateway_mono_ms, 50_000);
    assert_eq!(back, rows);
}

#[test]
fn policy_sync_ack_roundtrip() {
    use wardriver_packlink::{
        pack_policy_sync_ack, policy_sync_ack_error, policy_sync_ack_result,
        unpack_policy_sync_ack, PolicySyncAckV1,
    };
    let ack = PolicySyncAckV1 {
        policy_seq: 42,
        result: policy_sync_ack_result::OK,
        error_code: policy_sync_ack_error::NONE,
    };
    let wire = pack_policy_sync_ack(&ack, 7);
    let frame = unpack_frame(&wire).expect("frame");
    assert_eq!(frame.msg_type, MsgType::PolicySyncAck);
    assert_eq!(frame.seq, 7);
    let back = unpack_policy_sync_ack(frame.payload).expect("ack");
    assert_eq!(back, ack);
}

#[test]
fn pack_policy_sync_seq_sets_frame_seq() {
    use wardriver_packlink::pack_policy_sync_seq;
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
        roster: Vec::new(),
        dwell_ms: 100,
        chan_count: 0,
        channels: [0u8; CHANNELS_MAX],
        detection_disable_wifi_mask: 0,
        detection_disable_ble_mask: 0,
        ssid_watch: Vec::new(),
        remote_id_disable_mask: 0,
        ble_active_scan: false,
    };
    let wire = pack_policy_sync_seq(&policy, 99);
    let frame = unpack_frame(&wire).expect("frame");
    assert_eq!(frame.msg_type, MsgType::PolicySync);
    assert_eq!(frame.seq, 99);
}

#[test]
fn capture_cmd_roundtrip() {
    let cmd = CaptureCmdV1 {
        job_id: capture_job_id::WIFI_PROBE_PCAP,
        duration_sec: 60,
    };
    let wire = pack_capture_cmd(&cmd, 4);
    let frame = unpack_frame(&wire).expect("frame");
    assert_eq!(frame.msg_type, MsgType::CaptureCmd);
    let back = unpack_capture_cmd(frame.payload).expect("capture_cmd");
    assert_eq!(back, cmd);
}

#[test]
fn capture_status_roundtrip() {
    let status = CaptureStatusV1 {
        active: 1,
        hold: 1,
        remaining_ms: 45_000,
        frags_rx: 120,
        bytes_written: 8192,
        error_code: capture_error::NONE,
    };
    let wire = pack_capture_status(&status, 8);
    let frame = unpack_frame(&wire).expect("frame");
    assert_eq!(frame.msg_type, MsgType::CaptureStatus);
    let back = unpack_capture_status(frame.payload).expect("capture_status");
    assert_eq!(back, status);
}

#[test]
fn channel_plan_sync_roundtrip() {
    use wardriver_packlink::{
        band_mode, pack_channel_plan_sync, unpack_channel_plan_sync, ChannelPlanSyncV1,
    };
    let plan = ChannelPlanSyncV1 {
        schema_version: 1,
        revision: 9,
        include_2_4_ghz: true,
        include_5_ghz: true,
        include_dfs: false,
        dwell_ms: 200,
        band_mode: band_mode::MANUAL,
        channels_2_4: vec![1, 6, 11],
        channels_5: vec![36, 149],
        channels_5_dfs: Vec::new(),
        sniffers_2_4: vec![3],
        sniffers_5: vec![4],
        pins: vec![(3, 6)],
    };
    let wire = pack_channel_plan_sync(&plan, 12);
    let frame = unpack_frame(&wire).expect("frame");
    assert_eq!(frame.msg_type, MsgType::ChannelPlanSync);
    assert_eq!(frame.seq, 12);
    let back = unpack_channel_plan_sync(frame.payload).expect("plan");
    assert_eq!(back, plan);
}

#[test]
fn channel_plan_sync_ack_roundtrip() {
    use wardriver_packlink::{
        channel_plan_sync_ack_error, channel_plan_sync_ack_result, pack_channel_plan_sync_ack,
        unpack_channel_plan_sync_ack, ChannelPlanSyncAckV1,
    };
    let ack = ChannelPlanSyncAckV1 {
        plan_seq: 12,
        revision: 9,
        result: channel_plan_sync_ack_result::OK,
        error_code: channel_plan_sync_ack_error::NONE,
        hop_cycle_ms: 1800,
        unassigned_count: 2,
    };
    let wire = pack_channel_plan_sync_ack(&ack, 5);
    let frame = unpack_frame(&wire).expect("frame");
    assert_eq!(frame.msg_type, MsgType::ChannelPlanSyncAck);
    let back = unpack_channel_plan_sync_ack(frame.payload).expect("ack");
    assert_eq!(back, ack);
}

#[test]
fn fleet_unbind_notify_roundtrip() {
    use wardriver_packlink::{pack_fleet_unbind_notify, unpack_frame, MsgType};
    let wire = pack_fleet_unbind_notify(42);
    let frame = unpack_frame(&wire).expect("frame");
    assert_eq!(frame.msg_type, MsgType::FleetUnbindNotify);
    assert!(frame.payload.is_empty());
}

#[test]
fn sniffer_status_plan_ext_roundtrip() {
    use wardriver_packlink::SnifferStatusPlanExtV1;
    let row = SnifferStatusV1 {
        sniffer_id: 9,
        mac: [0x02, 0x11, 0x22, 0x33, 0x44, 0x99],
        last_seen_ms: 500,
        state: 1,
        role: 0,
        cpu_load_pct: 20,
        heap_avail_pct: 80,
        fleet_slot: 1,
        reboot_count: 0,
        adoption_state: 2,
        plan_ext: Some(SnifferStatusPlanExtV1 {
            five_ghz_capable: 1,
            assigned_band: 1,
            pinned_channel: 6,
            assigned_chan_count: 3,
            assigned_channels_preview: [1, 6, 11, 0],
        }),
        adopt_ext: None,
    };
    let wire = pack_sniffer_status(&row, 77_000, 2);
    let frame = unpack_frame(&wire).expect("frame");
    let (gateway_mono_ms, back) = unpack_sniffer_status_batch(frame.payload).expect("batch");
    assert_eq!(gateway_mono_ms, 77_000);
    assert_eq!(back.len(), 1);
    assert_eq!(back[0].plan_ext, row.plan_ext);
}

#[test]
fn sniffer_status_adopt_ext_roundtrip() {
    use wardriver_packlink::SnifferStatusAdoptExtV1;
    let row = SnifferStatusV1 {
        sniffer_id: 11,
        mac: [0x02, 0xDE, 0xAD, 0xBE, 0xEF, 0x01],
        last_seen_ms: 800,
        state: 1,
        role: 0,
        cpu_load_pct: 15,
        heap_avail_pct: 85,
        fleet_slot: 4,
        reboot_count: 1,
        adoption_state: 1,
        plan_ext: None,
        adopt_ext: Some(SnifferStatusAdoptExtV1 {
            adopt_progress: adopt_progress::RETRYING_WELCOME,
            adopt_reason: adopt_reason::SEND_FAILED,
        }),
    };
    let wire = pack_sniffer_status(&row, 88_000, 5);
    let frame = unpack_frame(&wire).expect("frame");
    let (gateway_mono_ms, back) = unpack_sniffer_status_batch(frame.payload).expect("batch");
    assert_eq!(gateway_mono_ms, 88_000);
    assert_eq!(back, vec![row]);
}
