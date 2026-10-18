//! Host tests for sniffer schedule signature hashing.

use wardriver_core::sniffer_schedule::{
    schedule_sig, SnifferScheduleSig, SNIFFER_SWEEP_MAX_DWELL_MS,
};
use wardriver_proto::CHANNELS_MAX;

fn sample_schedule() -> SnifferScheduleSig {
    let mut channels = [0u8; CHANNELS_MAX];
    channels[0] = 1;
    channels[1] = 6;
    channels[2] = 11;
    SnifferScheduleSig {
        control_channel: 11,
        dwell_ms: 200,
        chan_count: 3,
        channels,
    }
}

#[test]
fn schedule_sig_stable_for_same_schedule() {
    let a = sample_schedule();
    let b = sample_schedule();
    assert_eq!(schedule_sig(&a), schedule_sig(&b));
}

#[test]
fn schedule_sig_changes_when_chan_count_differs() {
    let mut a = sample_schedule();
    let mut b = sample_schedule();
    b.chan_count = 2;
    assert_ne!(schedule_sig(&a), schedule_sig(&b));
    let _ = &mut a;
}

#[test]
fn schedule_sig_changes_when_channel_list_differs() {
    let mut a = sample_schedule();
    let mut b = sample_schedule();
    b.channels[2] = 36;
    assert_ne!(schedule_sig(&a), schedule_sig(&b));
    let _ = &mut a;
}

#[test]
fn from_control_cmd_clamps_dwell_to_sweep_max() {
    use wardriver_proto::{ControlCmd, ControlCmdV1, CHANNELS_MAX};
    let mut channels = [0u8; CHANNELS_MAX];
    channels[0] = 1;
    let cmd = ControlCmdV1 {
        cmd: ControlCmd::SetChannelSchedule,
        cmd_seq: 1,
        control_channel: 11,
        dwell_ms: 10_000,
        chan_count: 1,
        channels,
        desired_role: 0,
        control_boot_id: 0,
        detection_disable_wifi_mask: 0,
        wardrive_suppressed: 0,
        time_set_seq: 0,
        ctrl_unix_ms: 0,
        detection_disable_ble_mask: 0,
        desired_scanning: 0,
        peer_slots_free: 0,
        peer_join_waiters: 0,
        remote_id_disable_mask: 0,
        ble_active_scan: 0,
    };
    let sig = SnifferScheduleSig::from_control_cmd(&cmd);
    assert_eq!(sig.dwell_ms, SNIFFER_SWEEP_MAX_DWELL_MS);
}
