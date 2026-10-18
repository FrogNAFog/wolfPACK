//! Write `.fleet/mgmt_ap.toml` at the repo root with default fleet management SoftAP credentials.
//!
//! `wardriver-shared/build.rs` reads this file at compile time (unless
//! `WARDRIVER_MGMT_AP_SSID` / `WARDRIVER_MGMT_AP_PASS` override). Idempotent: overwrites
//! the template on each run.

use std::fs;
use std::path::PathBuf;

const FLEET_DIR: &str = ".fleet";
const MGMT_AP_FILE: &str = "mgmt_ap.toml";

/// Default SSID/password baked into firmware when no fleet file or env pair is set.
const TEMPLATE: &str = r#"ssid = "wolfPACK"
password = "deflock123"
"#;

fn repo_root() -> PathBuf {
    let mut dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    dir.pop(); // wardriver-keygen -> repo root
    dir
}

fn main() {
    let root = repo_root();
    let fleet = root.join(FLEET_DIR);
    fs::create_dir_all(&fleet).expect("create .fleet");

    let path = fleet.join(MGMT_AP_FILE);
    fs::write(&path, TEMPLATE).expect("write mgmt_ap.toml");

    eprintln!("wardriver-keygen: wrote {}", path.display());
}
