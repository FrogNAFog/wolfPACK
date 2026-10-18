//! Host tests for `wardriver_core::lpr_camera_probe_ie` ordered TLV byte matcher.

mod support;

use support::old_lpr_camera_probe_ie_sig::legacy_probe_body_matches_lpr_camera_primary;
use wardriver_core::lpr_camera_probe_ie::{
    probe_body_matches_lpr_camera_primary, probe_req_mpdu_matches_lpr_camera_primary,
};

/// Minimal valid LPR-camera primary probe IE body (8-step sequence).
fn lpr_camera_primary_probe_body() -> Vec<u8> {
    let mut body = Vec::new();
    // 0: SSID wildcard
    body.extend_from_slice(&[0x00, 0x00]);
    // 2: supported rates (tag only — any length)
    body.extend_from_slice(&[0x02, 0x01, 0x82]);
    // 12: ERP (tag only)
    body.extend_from_slice(&[0x0c, 0x01, 0x00]);
    // 127: extended supported rates (tag only)
    body.extend_from_slice(&[0x7f, 0x01, 0x04]);
    // 221: Lite-On vendor
    body.extend_from_slice(&[0xdd, 0x07, 0x50, 0x6f, 0x9a, 0x16, 0x03, 0x01, 0x03]);
    // 45: HT capabilities (tag only)
    body.extend_from_slice(&[0x2d, 0x01, 0x00]);
    // 191: VHT capabilities (tag only)
    body.extend_from_slice(&[0xbf, 0x01, 0x00]);
    // 221: WPA vendor
    body.extend_from_slice(&[0xdd, 0x07, 0x00, 0x50, 0xf2, 0x08, 0x00, 0x00, 0x00]);
    body
}

/// ERP-first body matching legacy `FLOCK_PROBE_IE_SIG_PRIMARY_ALT_LINUX` (no tags 2 / 127).
fn lpr_camera_alt_linux_probe_body() -> Vec<u8> {
    let mut body = Vec::new();
    body.extend_from_slice(&[0x00, 0x00]);
    body.extend_from_slice(&[0x0c, 0x01, 0x00]);
    body.extend_from_slice(&[0xdd, 0x07, 0x50, 0x6f, 0x9a, 0x16, 0x03, 0x01, 0x03]);
    body.extend_from_slice(&[0x2d, 0x01, 0x00]);
    body.extend_from_slice(&[0xbf, 0x01, 0x00]);
    body.extend_from_slice(&[0xdd, 0x07, 0x00, 0x50, 0xf2, 0x08, 0x00, 0x00, 0x00]);
    body
}

/// IE body from `tests/fixtures/probeexample.pcapng` frame 1 (real LPR camera, wildcard probe).
fn probeexample_pcap_body() -> Vec<u8> {
    [
        0x00, 0x00, 0x01, 0x08, 0x0c, 0x12, 0x18, 0x24, 0x30, 0x48, 0x60, 0x6c, 0xdd, 0x07, 0x00,
        0x50, 0xf2, 0x08, 0x00, 0x1f, 0x00, 0x7f, 0x09, 0x04, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x40, 0x80, 0xdd, 0x07, 0x50, 0x6f, 0x9a, 0x16, 0x03, 0x01, 0x03, 0x2d, 0x1a, 0xef, 0x01,
        0x1f, 0xff, 0xff, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xbf, 0x0c, 0xb2, 0x01, 0x81, 0x33,
        0xfa, 0xff, 0x0c, 0x03, 0xfa, 0xff, 0x0c, 0x03, 0xdd, 0x07, 0x00, 0x50, 0xf2, 0x08, 0x00,
        0x00, 0x00,
    ]
    .to_vec()
}

/// Duplicate consecutive wildcard SSID headers before supported rates.
fn lpr_camera_duplicate_wildcard_ssid_probe_body() -> Vec<u8> {
    let mut body = vec![0x00, 0x00, 0x00, 0x00];
    body.extend_from_slice(&lpr_camera_primary_probe_body()[2..]);
    body
}

fn probe_req_mpdu_with_body(body: &[u8]) -> Vec<u8> {
    let mut mpdu = vec![0u8; 24];
    // FC: type=management (0), subtype=probe request (4) → 0x0040 LE
    mpdu[0] = 0x40;
    mpdu[1] = 0x00;
    mpdu.extend_from_slice(body);
    mpdu
}

fn assert_legacy_and_byte_matcher_agree(body: &[u8], case: &str) {
    let legacy = legacy_probe_body_matches_lpr_camera_primary(body);
    let byte = probe_body_matches_lpr_camera_primary(body);
    assert_eq!(
        legacy, byte,
        "legacy string oracle and byte matcher disagree on {case}"
    );
}

#[test]
fn primary_body_matches() {
    let body = lpr_camera_primary_probe_body();
    assert!(probe_body_matches_lpr_camera_primary(&body));
}

#[test]
fn primary_body_wrong_liteon_vendor() {
    let mut body = lpr_camera_primary_probe_body();
    // Flip one byte in Lite-On vendor payload (offset: SSID(2) + rates(3) + ERP(3) + ext(3) + tag/len(2) = 13)
    body[13] ^= 0x01;
    assert!(!probe_body_matches_lpr_camera_primary(&body));
}

#[test]
fn alt_linux_shape_legacy_and_byte_matcher_accept() {
    let body = lpr_camera_alt_linux_probe_body();
    assert!(legacy_probe_body_matches_lpr_camera_primary(&body));
    assert!(probe_body_matches_lpr_camera_primary(&body));
}

#[test]
fn primary_body_non_wildcard_ssid() {
    let mut body = lpr_camera_primary_probe_body();
    body[1] = 0x04;
    body.extend_from_slice(b"test");
    assert!(!probe_body_matches_lpr_camera_primary(&body));
}

#[test]
fn primary_body_truncated() {
    let body = lpr_camera_primary_probe_body();
    assert!(!probe_body_matches_lpr_camera_primary(
        &body[..body.len() - 2]
    ));
}

#[test]
fn primary_body_fcs_strip_retry() {
    let mut body = lpr_camera_primary_probe_body();
    body.extend_from_slice(&[0xde, 0xad, 0xbe, 0xef]);
    assert!(probe_body_matches_lpr_camera_primary(&body));
}

#[test]
fn mpdu_probe_req_matches() {
    let body = lpr_camera_primary_probe_body();
    let mpdu = probe_req_mpdu_with_body(&body);
    assert!(probe_req_mpdu_matches_lpr_camera_primary(&mpdu));
}

#[test]
fn mpdu_wrong_subtype_rejects() {
    let body = lpr_camera_primary_probe_body();
    let mut mpdu = probe_req_mpdu_with_body(&body);
    // Beacon subtype (8) → 0x0080
    mpdu[0] = 0x80;
    assert!(!probe_req_mpdu_matches_lpr_camera_primary(&mpdu));
}

#[test]
fn duplicate_wildcard_ssid_legacy_and_byte_matcher_accept() {
    let body = lpr_camera_duplicate_wildcard_ssid_probe_body();
    assert!(legacy_probe_body_matches_lpr_camera_primary(&body));
    assert!(probe_body_matches_lpr_camera_primary(&body));
}

#[test]
fn probeexample_pcap_legacy_and_byte_matcher_accept() {
    let body = probeexample_pcap_body();
    assert!(legacy_probe_body_matches_lpr_camera_primary(&body));
    assert!(probe_body_matches_lpr_camera_primary(&body));
}

#[test]
fn constructed_fixtures_legacy_byte_parity() {
    let cases = [
        ("primary_default", lpr_camera_primary_probe_body()),
        ("alt_linux", lpr_camera_alt_linux_probe_body()),
        (
            "duplicate_wildcard_ssid",
            lpr_camera_duplicate_wildcard_ssid_probe_body(),
        ),
        ("probeexample_pcap", probeexample_pcap_body()),
    ];
    for (name, body) in cases {
        assert_legacy_and_byte_matcher_agree(&body, name);
        let mut with_fcs = body.clone();
        with_fcs.extend_from_slice(&[0xde, 0xad, 0xbe, 0xef]);
        assert_legacy_and_byte_matcher_agree(&with_fcs, &format!("{name}+fcs"));
    }
}

#[test]
fn trailing_ie_after_signature_rejects_both_matchers() {
    let mut body = lpr_camera_primary_probe_body();
    // Extra vendor IE after the WPA tail — neither matcher should accept.
    body.extend_from_slice(&[0xdd, 0x04, 0x00, 0x50, 0xf2, 0x02]);
    assert!(!legacy_probe_body_matches_lpr_camera_primary(&body));
    assert!(!probe_body_matches_lpr_camera_primary(&body));
}
