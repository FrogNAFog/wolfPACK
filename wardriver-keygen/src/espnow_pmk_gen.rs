//! Generate a 16-byte ESP-NOW PMK and write `.fleet/espnow_pmk.bin` at the repo root.

use std::fs;
use std::path::PathBuf;

use rand::RngCore;
use sha2::{Digest, Sha256};

const PMK_LEN: usize = 16;
const FLEET_DIR: &str = ".fleet";
const PMK_FILE: &str = "espnow_pmk.bin";
const FINGERPRINT_FILE: &str = "espnow_pmk.fingerprint.txt";

fn repo_root() -> PathBuf {
    let mut dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    dir.pop(); // wardriver-keygen -> repo root
    dir
}

fn main() {
    let root = repo_root();
    let fleet = root.join(FLEET_DIR);
    fs::create_dir_all(&fleet).expect("create .fleet");

    let mut pmk = [0u8; PMK_LEN];
    rand::thread_rng().fill_bytes(&mut pmk);

    let pmk_path = fleet.join(PMK_FILE);
    fs::write(&pmk_path, &pmk).expect("write PMK");

    let hash = Sha256::digest(&pmk);
    let fp_path = fleet.join(FINGERPRINT_FILE);
    fs::write(&fp_path, format!("{hash:x}\n")).expect("write fingerprint");

    eprintln!(
        "wardriver-keygen: wrote {} ({} bytes)",
        pmk_path.display(),
        PMK_LEN
    );
    eprintln!(
        "wardriver-keygen: SHA256 fingerprint -> {}",
        fp_path.display()
    );
}
