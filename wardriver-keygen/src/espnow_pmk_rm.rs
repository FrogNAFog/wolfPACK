//! Remove `.fleet/espnow_pmk.bin` and fingerprint (idempotent).

use std::fs;
use std::path::PathBuf;

const FLEET_DIR: &str = ".fleet";
const PMK_FILE: &str = "espnow_pmk.bin";
const FINGERPRINT_FILE: &str = "espnow_pmk.fingerprint.txt";

fn repo_root() -> PathBuf {
    let mut dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    dir.pop();
    dir
}

fn main() {
    let fleet = repo_root().join(FLEET_DIR);
    let pmk = fleet.join(PMK_FILE);
    let fp = fleet.join(FINGERPRINT_FILE);

    let mut removed = false;
    if pmk.exists() {
        fs::remove_file(&pmk).expect("remove PMK file");
        eprintln!("wardriver-keygen: removed {}", pmk.display());
        removed = true;
    }
    if fp.exists() {
        fs::remove_file(&fp).expect("remove fingerprint");
        eprintln!("wardriver-keygen: removed {}", fp.display());
        removed = true;
    }
    if !removed {
        eprintln!("wardriver-keygen: nothing to remove (PMK files already absent)");
    }
}
