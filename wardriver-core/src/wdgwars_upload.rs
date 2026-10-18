//! wdgwars.pl upload HTTP response acceptance (aligned with PACK `backend/src/uploader.rs`).

use serde_json::Value;

const WDGWARS_DUPLICATE_NEEDLES: &[&str] = &[
    "already upload",
    "byte-for-byte",
    "byte for byte",
    "same byte",
    "duplicate",
    "same content",
    "identical content",
];

fn wdgwars_json_value(body: &str) -> Option<Value> {
    serde_json::from_str(body).ok()
}

fn wdgwars_json_ok(v: &Value) -> bool {
    v.get("ok").and_then(|o| o.as_bool()) == Some(true)
}

fn json_str_suggests_duplicate(s: &str) -> bool {
    let lower = s.to_ascii_lowercase();
    ["duplicate", "already", "uploaded", "conflict"]
        .iter()
        .any(|needle| lower.contains(needle))
}

fn wdgwars_json_duplicate(v: &Value) -> bool {
    if v.get("duplicate").and_then(|d| d.as_bool()) == Some(true) {
        return true;
    }
    if v.get("already_exists").and_then(|d| d.as_bool()) == Some(true) {
        return true;
    }
    for key in ["error", "code", "message"] {
        if let Some(s) = v.get(key).and_then(|x| x.as_str()) {
            if json_str_suggests_duplicate(s) {
                return true;
            }
        }
    }
    false
}

fn wdgwars_body_suggests_duplicate(body: &str) -> bool {
    if body.is_empty() {
        return false;
    }
    let lower = body.to_ascii_lowercase();
    WDGWARS_DUPLICATE_NEEDLES
        .iter()
        .any(|needle| lower.contains(needle))
}

/// HTTP 409 with duplicate semantics — retry after a prior successful ingest.
pub fn wdgwars_duplicate_accepted(status: u16, body: &str) -> bool {
    if status != 409 {
        return false;
    }
    if wdgwars_body_suggests_duplicate(body) {
        return true;
    }
    wdgwars_json_value(body).is_some_and(|v| wdgwars_json_duplicate(&v))
}

/// True when wdgwars accepted the upload (including duplicate-as-success).
pub fn wdgwars_upload_accepted(status: u16, body: &str) -> bool {
    if wdgwars_duplicate_accepted(status, body) {
        return true;
    }
    if (200..300).contains(&status) {
        if let Some(v) = wdgwars_json_value(body) {
            return wdgwars_json_ok(&v) || wdgwars_json_duplicate(&v);
        }
        return true;
    }
    if (500..600).contains(&status) {
        return wdgwars_json_value(body).is_some_and(|v| wdgwars_json_ok(&v));
    }
    false
}

/// Short body preview for logs when a non-2xx response is still accepted.
pub fn wdgwars_body_preview(body: &str) -> String {
    const MAX: usize = 200;
    if body.len() <= MAX {
        body.to_string()
    } else {
        format!("{}…", &body[..MAX])
    }
}

/// WiGLE upload JSON uses top-level `"success": true` on HTTP 2xx.
pub fn wigle_upload_accepted(status: u16, body: &str) -> bool {
    if !(200..300).contains(&status) {
        return false;
    }
    body.contains("\"success\":true") || body.contains("\"success\": true")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_200_json_ok() {
        assert!(wdgwars_upload_accepted(200, r#"{"ok":true}"#));
    }

    #[test]
    fn accepts_200_empty_body() {
        assert!(wdgwars_upload_accepted(200, ""));
    }

    #[test]
    fn rejects_200_json_not_ok() {
        assert!(!wdgwars_upload_accepted(200, r#"{"ok":false}"#));
    }

    #[test]
    fn rejects_409_empty_body() {
        assert!(!wdgwars_upload_accepted(409, ""));
    }

    #[test]
    fn accepts_409_typo_prose_duplicate() {
        let body = "This file wasx already uploaded on 2026-05-29 18:57:31. Same byte-for-byte ccontent. Skip or modify the file before retying.";
        assert!(wdgwars_upload_accepted(409, body));
    }

    #[test]
    fn accepts_409_json_duplicate_flag() {
        assert!(wdgwars_upload_accepted(409, r#"{"duplicate":true}"#));
    }

    #[test]
    fn rejects_409_unrelated_conflict() {
        assert!(!wdgwars_upload_accepted(
            409,
            "version conflict: resource locked"
        ));
    }

    #[test]
    fn accepts_502_with_json_ok() {
        assert!(wdgwars_upload_accepted(502, r#"{"ok":true}"#));
    }

    #[test]
    fn rejects_502_empty() {
        assert!(!wdgwars_upload_accepted(502, ""));
    }

    #[test]
    fn rejects_502_html() {
        assert!(!wdgwars_upload_accepted(
            502,
            "<html><body>Bad Gateway</body></html>"
        ));
    }

    #[test]
    fn accepts_201_ok_and_duplicate() {
        assert!(wdgwars_upload_accepted(
            201,
            r#"{"ok":true,"duplicate":true}"#
        ));
    }
}
