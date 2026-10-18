//! SSID watch patterns — pure Rust, no esp-idf.
//!
//! Supports exact-match SSIDs and tagged pattern rules (PREFIX:, PREFIX_LEN:, CONTAINS:).
//! The combined [`SsidWatch`] structure replaces the previous `HashSet<String>` watchlist
//! so both match strategies coexist in a single user-editable list.

use std::collections::HashSet;

/// Pattern-based SSID watch rule (parsed from tagged lines).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SsidWatchRule {
    /// SSID starts with `prefix` (e.g. `PREFIX:MLG`).
    Prefix { prefix: String },
    /// SSID starts with `prefix` AND has exactly `len` characters (e.g. `PREFIX_LEN:MLG,8`).
    PrefixLen { prefix: String, len: u8 },
    /// SSID contains `substring` anywhere (e.g. `CONTAINS:AXON`).
    Contains { substring: String },
}

impl SsidWatchRule {
    /// Match against a normalized SSID string.
    pub fn matches(&self, ssid: &str) -> bool {
        match self {
            Self::Prefix { prefix } => ssid.starts_with(prefix),
            Self::PrefixLen { prefix, len } => {
                ssid.starts_with(prefix) && ssid.len() == (*len as usize)
            }
            Self::Contains { substring } => ssid.contains(substring),
        }
    }
}

/// A single parsed entry from the SSID watch list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SsidWatchEntry {
    /// Exact SSID match (plain line with no tag).
    Exact(String),
    /// Pattern rule (tagged line).
    Pattern(SsidWatchRule),
}

/// Combined SSID watch list: exact-match SSIDs + pattern rules.
#[derive(Debug, Clone, Default)]
pub struct SsidWatch {
    /// Exact-match SSIDs.
    pub exact: HashSet<String>,
    /// Tagged pattern rules.
    pub patterns: Vec<SsidWatchRule>,
}

impl SsidWatch {
    /// Create an empty watch list.
    pub fn new() -> Self {
        Self {
            exact: HashSet::new(),
            patterns: Vec::new(),
        }
    }

    /// True when both the exact set and patterns vec are empty.
    pub fn is_empty(&self) -> bool {
        self.exact.is_empty() && self.patterns.is_empty()
    }

    /// Match a normalized SSID against exact entries first, then patterns.
    pub fn matches(&self, ssid: &str) -> bool {
        if self.exact.contains(ssid) {
            return true;
        }
        self.patterns.iter().any(|rule| rule.matches(ssid))
    }

    /// Build from a list of raw strings (as received over the wire).
    pub fn from_strings(ssids: Vec<String>) -> Self {
        let mut exact = HashSet::new();
        let mut patterns = Vec::new();
        for s in ssids {
            match parse_ssid_watch_line(&s) {
                SsidWatchEntry::Exact(e) => {
                    let _ = exact.insert(e);
                }
                SsidWatchEntry::Pattern(p) => {
                    patterns.push(p);
                }
            }
        }
        Self { exact, patterns }
    }
}

/// Parse a single watchlist line into either an exact-match String or a pattern rule.
///
/// Order matters: `PREFIX_LEN` is checked before `PREFIX` so `PREFIX_LEN:MLG,8` is not
/// misclassified as a `PREFIX:` rule with payload `LEN:MLG,8`.
pub fn parse_ssid_watch_line(line: &str) -> SsidWatchEntry {
    // PREFIX_LEN before PREFIX (longer prefix wins).
    if let Some(rest) = line.strip_prefix("PREFIX_LEN:") {
        if let Some(comma_pos) = rest.rfind(',') {
            let prefix = &rest[..comma_pos];
            if let Ok(len) = rest[comma_pos + 1..].parse::<u8>() {
                return SsidWatchEntry::Pattern(SsidWatchRule::PrefixLen {
                    prefix: prefix.to_owned(),
                    len,
                });
            }
        }
        // Malformed PREFIX_LEN falls through to exact match.
    }

    if let Some(rest) = line.strip_prefix("PREFIX:") {
        return SsidWatchEntry::Pattern(SsidWatchRule::Prefix {
            prefix: rest.to_owned(),
        });
    }

    if let Some(rest) = line.strip_prefix("CONTAINS:") {
        return SsidWatchEntry::Pattern(SsidWatchRule::Contains {
            substring: rest.to_owned(),
        });
    }

    // No tag — exact match (existing behavior).
    SsidWatchEntry::Exact(line.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── parse_ssid_watch_line ──

    #[test]
    fn parse_exact_match() {
        let entry = parse_ssid_watch_line("MyNetwork");
        assert_eq!(entry, SsidWatchEntry::Exact("MyNetwork".to_owned()));
    }

    #[test]
    fn parse_prefix_tag() {
        let entry = parse_ssid_watch_line("PREFIX:MLG");
        assert_eq!(
            entry,
            SsidWatchEntry::Pattern(SsidWatchRule::Prefix {
                prefix: "MLG".to_owned(),
            })
        );
    }

    #[test]
    fn parse_prefix_len_tag() {
        let entry = parse_ssid_watch_line("PREFIX_LEN:MLG,8");
        assert_eq!(
            entry,
            SsidWatchEntry::Pattern(SsidWatchRule::PrefixLen {
                prefix: "MLG".to_owned(),
                len: 8,
            })
        );
    }

    #[test]
    fn parse_contains_tag() {
        let entry = parse_ssid_watch_line("CONTAINS:AXON");
        assert_eq!(
            entry,
            SsidWatchEntry::Pattern(SsidWatchRule::Contains {
                substring: "AXON".to_owned(),
            })
        );
    }

    #[test]
    fn parse_prefix_len_falls_back_to_exact_on_malformed() {
        // No comma → treated as exact string.
        let entry = parse_ssid_watch_line("PREFIX_LEN:MLG");
        assert_eq!(entry, SsidWatchEntry::Exact("PREFIX_LEN:MLG".to_owned()));
    }

    #[test]
    fn parse_prefix_len_falls_back_to_exact_on_bad_len() {
        // Non-numeric length → treated as exact string.
        let entry = parse_ssid_watch_line("PREFIX_LEN:MLG,abc");
        assert_eq!(
            entry,
            SsidWatchEntry::Exact("PREFIX_LEN:MLG,abc".to_owned())
        );
    }

    // ── SsidWatchRule::matches ──

    #[test]
    fn prefix_len_match_exact() {
        let rule = SsidWatchRule::PrefixLen {
            prefix: "MLG".to_owned(),
            len: 8,
        };
        assert!(rule.matches("MLG00123"));
        assert!(rule.matches("MLG99999"));
    }

    #[test]
    fn prefix_len_no_match_short() {
        let rule = SsidWatchRule::PrefixLen {
            prefix: "MLG".to_owned(),
            len: 8,
        };
        assert!(!rule.matches("MLG12"));
    }

    #[test]
    fn prefix_len_no_match_long() {
        let rule = SsidWatchRule::PrefixLen {
            prefix: "MLG".to_owned(),
            len: 8,
        };
        assert!(!rule.matches("MLG123456"));
    }

    #[test]
    fn prefix_len_no_match_wrong_prefix() {
        let rule = SsidWatchRule::PrefixLen {
            prefix: "MLG".to_owned(),
            len: 8,
        };
        assert!(!rule.matches("OtherMLG"));
    }

    #[test]
    fn prefix_len_not_digit_only() {
        // PREFIX_LEN does not restrict character class.
        let rule = SsidWatchRule::PrefixLen {
            prefix: "MLG".to_owned(),
            len: 8,
        };
        assert!(rule.matches("MLG-0012"));
        assert!(rule.matches("MLGabcde"));
    }

    #[test]
    fn prefix_match_any_suffix() {
        let rule = SsidWatchRule::Prefix {
            prefix: "MLG".to_owned(),
        };
        assert!(rule.matches("MLG00123"));
        assert!(rule.matches("MLG99"));
        assert!(rule.matches("MLG"));
        assert!(!rule.matches("OtherMLG00123"));
    }

    #[test]
    fn contains_match_anywhere() {
        let rule = SsidWatchRule::Contains {
            substring: "AXON".to_owned(),
        };
        assert!(rule.matches("AXON-01"));
        assert!(rule.matches("MyAXON"));
        assert!(rule.matches("AXON"));
        assert!(!rule.matches("axon")); // case sensitive
    }

    // ── SsidWatch::matches (combined) ──

    #[test]
    fn combined_exact_match() {
        let mut watch = SsidWatch::new();
        watch.exact.insert("HomeWiFi".to_owned());
        assert!(watch.matches("HomeWiFi"));
        assert!(!watch.matches("OtherWiFi"));
    }

    #[test]
    fn combined_pattern_match() {
        let mut watch = SsidWatch::new();
        watch.patterns.push(SsidWatchRule::PrefixLen {
            prefix: "MLG".to_owned(),
            len: 8,
        });
        assert!(watch.matches("MLG00123"));
        assert!(!watch.matches("MLG12"));
    }

    #[test]
    fn combined_exact_and_patterns() {
        let mut watch = SsidWatch::new();
        watch.exact.insert("HomeWiFi".to_owned());
        watch.patterns.push(SsidWatchRule::Prefix {
            prefix: "MLG".to_owned(),
        });
        assert!(watch.matches("HomeWiFi"));
        assert!(watch.matches("MLG99999"));
        assert!(!watch.matches("Other"));
    }

    #[test]
    fn combined_empty_returns_false() {
        let watch = SsidWatch::new();
        assert!(!watch.matches("Anything"));
        assert!(watch.is_empty());
    }

    #[test]
    fn is_empty_false_with_patterns_only() {
        let mut watch = SsidWatch::new();
        watch.patterns.push(SsidWatchRule::Prefix {
            prefix: "X".to_owned(),
        });
        assert!(!watch.is_empty());
    }

    // ── SsidWatch::from_strings ──

    #[test]
    fn from_strings_mixed() {
        let watch = SsidWatch::from_strings(vec![
            "HomeWiFi".to_owned(),
            "PREFIX:MLG".to_owned(),
            "CONTAINS:AXON".to_owned(),
        ]);
        assert_eq!(watch.exact.len(), 1);
        assert!(watch.exact.contains("HomeWiFi"));
        assert_eq!(watch.patterns.len(), 2);
        assert!(watch.matches("HomeWiFi"));
        assert!(watch.matches("MLG12345"));
        assert!(watch.matches("MyAXONDevice"));
    }

    #[test]
    fn from_strings_dedup_exact() {
        let watch = SsidWatch::from_strings(vec![
            "HomeWiFi".to_owned(),
            "HomeWiFi".to_owned(),
            "Other".to_owned(),
        ]);
        assert_eq!(watch.exact.len(), 2);
        assert_eq!(watch.patterns.len(), 0);
    }
}
