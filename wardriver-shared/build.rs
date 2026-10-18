//! ESP-IDF env + generate `OUT_DIR/espnow_pmk.rs` for ESP-NOW encryption (see `wardriver-keygen`).
//! Also emits `OUT_DIR/mgmt_ap.rs` for compile-time fleet management SoftAP credentials.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

fn main() {
    embuild::espidf::sysenv::output();

    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"));
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let workspace_root = manifest_dir
        .parent()
        .expect("wardriver-shared workspace root");

    println!("cargo:rerun-if-changed=build.rs");
    generate_mgmt_ap_rs(&out_dir, &workspace_root);

    let pmk_rs = out_dir.join("espnow_pmk.rs");

    // Optional build override: a 16-byte PMK file at a custom path (used by the
    // release packaging so published zips get their own rotated key and never
    // contain the operator's private `.fleet/espnow_pmk.bin`).
    let fleet_pmk = match std::env::var("WARDRIVER_FLEET_PMK_FILE") {
        Ok(p) if !p.is_empty() => {
            println!("cargo:rerun-if-env-changed=WARDRIVER_FLEET_PMK_FILE");
            PathBuf::from(&p)
        }
        _ => workspace_root.join(".fleet").join("espnow_pmk.bin"),
    };

    let pmk: Vec<u8> = {
        println!("cargo:rerun-if-changed={}", fleet_pmk.display());
        let v = fs::read(&fleet_pmk).expect(
            "read .fleet/espnow_pmk.bin — PMK missing. Generate: ./tools/espnow-pmk gen. See README: Fleet ESP-NOW PMK.",
        );
        if v.len() != 16 {
            panic!(
                "{} must be exactly 16 bytes, got {} — run `./tools/espnow-pmk gen`",
                fleet_pmk.display(),
                v.len()
            );
        }
        v
    };

    let mut body =
        String::from("/// Compile-time ESP-NOW PMK (16 bytes). From the fleet PMK file supplied at build time.\n");
    // static, not const: a const can be fully inlined as immediates, leaving
    // no contiguous .rodata copy for the webflash PMK offset search.
    body.push_str("pub static ESPNOW_PMK_BYTES: [u8; 16] = [");
    for (i, b) in pmk.iter().enumerate() {
        if i > 0 {
            body.push(',');
        }
        body.push_str(&format!("{}", b));
    }
    body.push_str("];\n");
    fs::write(&pmk_rs, body).expect("write espnow_pmk.rs");
}

/// Precedence: env pair (both set) → `.fleet/mgmt_ap.toml` → `wolfPACK` / `deflock123`.
fn generate_mgmt_ap_rs(out_dir: &Path, workspace_root: &Path) {
    let fleet_toml = workspace_root.join(".fleet").join("mgmt_ap.toml");

    println!("cargo:rerun-if-env-changed=WARDRIVER_MGMT_AP_SSID");
    println!("cargo:rerun-if-env-changed=WARDRIVER_MGMT_AP_PASS");
    println!("cargo:rerun-if-changed={}", fleet_toml.display());

    let (ssid, pass) = resolve_mgmt_ap(&fleet_toml);
    validate_mgmt_ap_ssid(&ssid);
    validate_mgmt_ap_pass(&pass);

    let body = format!(
        "pub const FLEET_MGMT_AP_SSID: &str = {:?};\npub const FLEET_MGMT_AP_PASS: &str = {:?};\n",
        ssid, pass
    );
    fs::write(out_dir.join("mgmt_ap.rs"), body).expect("write mgmt_ap.rs");
}

fn resolve_mgmt_ap(fleet_toml: &Path) -> (String, String) {
    let ssid_env = env::var("WARDRIVER_MGMT_AP_SSID").ok();
    let pass_env = env::var("WARDRIVER_MGMT_AP_PASS").ok();

    // Both env vars required; a lone SSID or password falls through to file/defaults.
    if let (Some(ssid), Some(pass)) = (ssid_env, pass_env) {
        return (ssid, pass);
    }

    if fleet_toml.exists() {
        let content = fs::read_to_string(fleet_toml).expect("read .fleet/mgmt_ap.toml");
        let table: toml::Table =
            toml::from_str(&content).expect("parse .fleet/mgmt_ap.toml as TOML table");
        let ssid = table
            .get("ssid")
            .and_then(|v| v.as_str())
            .expect(".fleet/mgmt_ap.toml: missing or invalid `ssid` (string)")
            .to_string();
        let pass = table
            .get("password")
            .and_then(|v| v.as_str())
            .expect(".fleet/mgmt_ap.toml: missing or invalid `password` (string)")
            .to_string();
        return (ssid, pass);
    }

    ("wolfPACK".into(), "deflock123".into())
}

fn validate_mgmt_ap_ssid(ssid: &str) {
    if ssid.is_empty() {
        panic!("fleet management AP SSID must be non-empty");
    }
    let nbytes = ssid.len();
    if nbytes > 32 {
        panic!("fleet management AP SSID must be at most 32 UTF-8 bytes, got {nbytes}");
    }
}

fn validate_mgmt_ap_pass(pass: &str) {
    let nbytes = pass.as_bytes().len();
    if !(8..=63).contains(&nbytes) {
        panic!("fleet management AP password must be 8-63 UTF-8 bytes, got {nbytes}");
    }
}
