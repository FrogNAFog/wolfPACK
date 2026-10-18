//! ESP-IDF env + generate `OUT_DIR/espnow_pmk.rs` for ESP-NOW encryption (see `wardriver-keygen`).

use std::env;
use std::fs;
use std::path::PathBuf;

fn main() {
    // esp-idf-sys sdkconfig is fixed before this build.rs runs (Cargo.toml metadata / env).
    println!("cargo:rerun-if-changed=idf_extra/bindings_extra.h");

    embuild::espidf::sysenv::output();

    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"));
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let pmk_rs = out_dir.join("espnow_pmk.rs");

    // Optional build override: a 16-byte PMK file at a custom path (used by the
    // release packaging so published zips get their own rotated key and never
    // contain the operator's private `.fleet/espnow_pmk.bin`).
    let fleet_pmk = match std::env::var("WARDRIVER_FLEET_PMK_FILE") {
        Ok(p) if !p.is_empty() => {
            println!("cargo:rerun-if-env-changed=WARDRIVER_FLEET_PMK_FILE");
            PathBuf::from(&p)
        }
        _ => manifest_dir.join(".fleet").join("espnow_pmk.bin"),
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

    println!("cargo:rerun-if-changed=build.rs");
}
