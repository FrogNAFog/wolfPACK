use wardriver_fleet_realtime::sniffer_lifecycle::{hello_gate, BootTracker, HelloGate};

#[test]
fn control_and_gateway_hello_gate_parity() {
    let mut boot = BootTracker::new();
    assert!(!boot.note_boot(9, 0x100));
    let rebooted = boot.note_boot(9, 0x200);
    assert!(rebooted);

    assert_eq!(
        hello_gate(false, Some(1), false),
        Some(HelloGate::DropNotRostered)
    );
    assert_eq!(
        hello_gate(true, Some(3), rebooted),
        Some(HelloGate::Accept {
            fleet_slot: 3,
            rebooted: true
        })
    );
}

#[test]
fn boot_tracker_uplink_mismatch() {
    let mut boot = BootTracker::new();
    boot.note_boot(4, 0x10);
    assert!(boot.uplink_boot_mismatch(4, 0x20));
    assert!(!boot.uplink_boot_mismatch(4, 0x10));
}
