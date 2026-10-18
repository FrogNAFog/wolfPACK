use wardriver_core::fleet_roster::{
    decode_roster_blob, encode_roster_blob, next_free_fleet_slot, roster_adopt_entry,
    roster_is_adopted, roster_remove_entry, FleetRosterEntry, FLEET_MAX_SNIFFERS,
    ROSTER_BLOB_VERSION,
};

#[test]
fn next_free_slot_skips_used() {
    assert_eq!(next_free_fleet_slot(&[]), Some(1));
    assert_eq!(next_free_fleet_slot(&[1, 3]), Some(2));
    let all: Vec<u8> = (1..=FLEET_MAX_SNIFFERS as u8).collect();
    assert_eq!(next_free_fleet_slot(&all), None);
}

#[test]
fn roster_adopt_and_remove() {
    let mac_a = [0x02, 0, 0, 0, 0, 1];
    let mac_b = [0x02, 0, 0, 0, 0, 2];
    let mut entries = Vec::new();
    assert_eq!(roster_adopt_entry(&mut entries, 10, mac_a), Some(1));
    assert_eq!(roster_adopt_entry(&mut entries, 10, mac_a), Some(1));
    assert_eq!(roster_adopt_entry(&mut entries, 11, mac_b), Some(2));
    assert!(roster_is_adopted(&entries, 10, &mac_a));
    assert!(!roster_is_adopted(&entries, 10, &mac_b));
    assert!(roster_remove_entry(&mut entries, 10));
    assert!(!roster_is_adopted(&entries, 10, &mac_a));
    assert_eq!(entries.len(), 1);
}

#[test]
fn roster_blob_roundtrip() {
    let entries = vec![
        FleetRosterEntry {
            sniffer_id: 42,
            mac: [0xde, 0xad, 0xbe, 0xef, 0x00, 0x01],
            fleet_slot: 3,
        },
        FleetRosterEntry {
            sniffer_id: 7,
            mac: [0x02, 0x11, 0x22, 0x33, 0x44, 0x55],
            fleet_slot: 1,
        },
    ];
    let blob = encode_roster_blob(&entries);
    assert_eq!(blob[0], ROSTER_BLOB_VERSION);
    assert_eq!(blob[1], 2);
    let back = decode_roster_blob(&blob);
    assert_eq!(back, entries);
}

#[test]
fn roster_blob_rejects_bad_version() {
    assert!(decode_roster_blob(&[0, 1]).is_empty());
    assert!(decode_roster_blob(&[]).is_empty());
}

#[test]
fn full_roster_48_slots() {
    assert_eq!(FLEET_MAX_SNIFFERS, 48);

    let mut entries = Vec::new();
    for id in 1..=48u32 {
        let mac = [0x02, (id >> 16) as u8, (id >> 8) as u8, id as u8, 0, 0];
        let slot = roster_adopt_entry(&mut entries, id, mac).expect("adopt slot");
        assert_eq!(slot, id as u8);
        assert!(roster_is_adopted(&entries, id, &mac));
    }
    assert_eq!(entries.len(), 48);
    assert_eq!(
        next_free_fleet_slot(&entries.iter().map(|e| e.fleet_slot).collect::<Vec<_>>()),
        None
    );
    assert_eq!(
        roster_adopt_entry(&mut entries, 99, [0x02, 0, 0, 0, 0, 99]),
        None
    );

    let blob = encode_roster_blob(&entries);
    assert_eq!(blob[0], ROSTER_BLOB_VERSION);
    assert_eq!(blob[1], 48);
    let back = decode_roster_blob(&blob);
    assert_eq!(back.len(), 48);
    for (orig, decoded) in entries.iter().zip(back.iter()) {
        assert_eq!(orig, decoded);
    }

    let mut over = blob.clone();
    over[1] = 49;
    assert!(decode_roster_blob(&over).is_empty());
}
