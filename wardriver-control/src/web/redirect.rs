//! HTTP 303 See Other — POST handlers redirect instead of rebuilding HTML pages.

use embedded_io::ErrorType;
use esp_idf_svc::http::server::{Connection, Request};

/// Empty body; browser follows `Location` (keeps httpd off SD / large String work).
pub(crate) fn see_other<C>(req: Request<C>, location: &str) -> Result<(), anyhow::Error>
where
    C: ErrorType + Connection,
{
    req.into_response(303, Some("See Other"), &[("Location", location)])
        .map_err(|e| anyhow::anyhow!("{e:?}"))?;
    Ok(())
}

/// Percent-encode a short user-visible message for a query `detail=` parameter.
pub(crate) fn query_param_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars().take(120) {
        match c {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' => out.push(c),
            ' ' => out.push_str("%20"),
            _ => {
                for b in c.encode_utf8(&mut [0u8; 4]).bytes() {
                    out.push('%');
                    out.push(char::from_digit((b >> 4) as u32, 16).unwrap());
                    out.push(char::from_digit((b & 0xf) as u32, 16).unwrap());
                }
            }
        }
    }
    out
}
