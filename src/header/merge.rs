//! Explicit, field-aware merging of two [`HeaderMap`]s.
//!
//! Plainly concatenating two sets of HTTP headers is not always legal: some
//! fields are defined as comma separated lists (e.g. `Accept`), some may only
//! carry a single value that all senders must agree on (`Content-Length`),
//! some fields must never be combined into one value (`Set-Cookie`), and the
//! `Connection` header names per-message hop-by-hop fields that are filtered
//! when a message is forwarded.
//!
//! This module implements [`HeaderMap::merge`], which classifies every field
//! and applies the corresponding combination rule. A brand new `HeaderMap` is
//! only produced when **every** field validates; the two input maps are never
//! mutated and the ordinary [`HeaderMap::append`] behavior is untouched.
//!
//! # Field classes
//!
//! | Class            | Combination rule |
//! |------------------|------------------|
//! | list fields      | all values are combined into one comma separated value |
//! | `Set-Cookie` and other multi-value non-list fields | every value is kept as a separate header entry |
//! | `Content-Length` | every value (across both maps) must parse to the same integer |
//! | single-value fields | all values must be byte-for-byte identical |
//! | unknown fields   | treated conservatively: every value is kept as a separate entry |
//!
//! [`HeaderName`] is case insensitive, so a field that appears as
//! `Content-Type` in one map and `content-type` in the other shares one
//! identity and never results in two keys.
//!
//! # Local verification
//!
//! These small, scoped commands exercise the merge behavior without touching
//! the rest of the workspace:
//!
//! ```sh
//! cargo test --lib header::merge
//! cargo test --test merge_headers
//! cargo run --example merge_headers
//! cargo test --doc header::merge
//! ```
//!
//! # Example
//!
//! The five-header example merges two `Set-Cookie` headers (kept apart), an
//! `Accept` list (combined), matching `Content-Length` values, a
//! `Connection` token that causes the named field to be filtered, and an
//! unknown `x-custom` field whose multiple values are preserved:
//!
//! ```
//! use http::HeaderMap;
//!
//! let mut a = HeaderMap::new();
//! a.append("set-cookie", "a=1".parse().unwrap());
//! a.insert("accept", "text/html".parse().unwrap());
//! a.insert("content-length", "42".parse().unwrap());
//! a.insert("connection", "keep-alive".parse().unwrap());
//! a.append("x-custom", "one".parse().unwrap());
//!
//! let mut b = HeaderMap::new();
//! b.append("set-cookie", "b=2".parse().unwrap());
//! b.insert("accept", "application/json".parse().unwrap());
//! b.insert("content-length", "42".parse().unwrap());
//! b.insert("keep-alive", "timeout=5".parse().unwrap());
//! b.append("x-custom", "two".parse().unwrap());
//!
//! let merged = a.merge(&b).unwrap();
//!
//! // Two Set-Cookie values survive as two entries (no comma joining).
//! assert_eq!(merged.get_all("set-cookie").iter().count(), 2);
//! // List fields are combined.
//! assert_eq!(merged["accept"], "text/html, application/json");
//! // Agreeing single values collapse to one.
//! assert_eq!(merged["content-length"], "42");
//! // The Connection-named hop-by-hop field is filtered...
//! assert!(!merged.contains_key("keep-alive"));
//! // ...while Connection itself remains.
//! assert_eq!(merged["connection"], "keep-alive");
//! // Unknown fields keep every value.
//! assert_eq!(merged.get_all("x-custom").iter().count(), 2);
//! ```

use std::fmt;

use super::name::HeaderName;
use super::{HeaderMap, HeaderValue, ACCEPT, CONNECTION, CONTENT_LENGTH, SET_COOKIE};

/// Configuration for how [`HeaderMap::merge`] treats hop-by-hop fields.
///
/// The default policy follows the forwarding rules of [RFC 7230 section
/// 6.1]: the standard hop-by-hop fields are removed and, in addition, every
/// field named by a `Connection` header token is removed. The `Connection`
/// header itself is always kept and merged.
///
/// [RFC 7230 section 6.1]: https://www.rfc-editor.org/rfc/rfc7230#section-6.1
#[derive(Clone, Debug)]
pub struct MergePolicy {
    /// Remove the fixed set of standard hop-by-hop headers (`Keep-Alive`,
    /// `Proxy-Connection`, `TE`, `Trailer`, `Transfer-Encoding`,
    /// `Upgrade`).
    pub remove_standard_hop_by_hop: bool,

    /// Remove every header field named by a token listed in a `Connection`
    /// header value.
    pub filter_connection_named: bool,
}

impl Default for MergePolicy {
    fn default() -> Self {
        Self {
            remove_standard_hop_by_hop: true,
            filter_connection_named: true,
        }
    }
}

impl MergePolicy {
    /// The RFC 7230 forwarding policy: standard hop-by-hop headers and
    /// `Connection`-named headers are filtered out.
    pub fn rfc_forwarding() -> Self {
        Self::default()
    }

    /// Retain every field; only the value-combination rules are applied.
    pub fn retain_all() -> Self {
        Self {
            remove_standard_hop_by_hop: false,
            filter_connection_named: false,
        }
    }
}

/// Error returned when two [`HeaderMap`]s cannot be merged.
#[derive(Debug)]
#[non_exhaustive]
pub enum MergeError {
    /// A single-value field carries disagreeing values.
    Conflict {
        /// The field whose values disagree.
        name: HeaderName,
    },
    /// `Content-Length` values disagree or one of them is not a valid
    /// decimal length.
    InvalidContentLength {
        /// The offending raw value, copied for reporting.
        value: HeaderValue,
    },
    /// The merged map would exceed the maximum number of header entries.
    TooLarge,
}

impl fmt::Display for MergeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MergeError::Conflict { name } => {
                write!(f, "conflicting values for header field `{}`", name)
            }
            MergeError::InvalidContentLength { value } => write!(
                f,
                "invalid or conflicting content-length: {:?}",
                value.to_str().unwrap_or("<non-ascii>")
            ),
            MergeError::TooLarge => f.write_str("merged header map exceeds maximum size"),
        }
    }
}

impl std::error::Error for MergeError {}

/// How a field's values are combined when two maps are merged.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    /// Combine every value into one comma separated value.
    CommaList,
    /// Keep every value as a separate header entry (`Set-Cookie`).
    MultiKeep,
    /// Keep every value as a separate entry (unknown fields; conservative).
    UnknownKeep,
    /// All values must parse to the same integer (`Content-Length`).
    ContentLength,
    /// All values must be byte-for-byte identical.
    Single,
}

/// Fields whose ABNF permits `1#element`: combining values with commas is
/// semantics-preserving.
const COMMA_LIST_FIELDS: &[HeaderName] = &[
    super::ACCEPT_CHARSET,
    super::ACCEPT_ENCODING,
    super::ACCEPT_LANGUAGE,
    super::ACCESS_CONTROL_ALLOW_HEADERS,
    super::ACCESS_CONTROL_ALLOW_METHODS,
    super::ACCESS_CONTROL_EXPOSE_HEADERS,
    super::ACCESS_CONTROL_REQUEST_HEADERS,
    super::ALLOW,
    super::ALT_SVC,
    super::CACHE_CONTROL,
    super::CACHE_STATUS,
    super::CDN_CACHE_CONTROL,
    CONNECTION,
    super::CONTENT_ENCODING,
    super::CONTENT_LANGUAGE,
    super::EXPECT,
    super::FORWARDED,
    super::IF_MATCH,
    super::IF_NONE_MATCH,
    super::LINK,
    super::PRAGMA,
    super::PROXY_AUTHENTICATE,
    super::SEC_WEBSOCKET_EXTENSIONS,
    super::SEC_WEBSOCKET_PROTOCOL,
    super::TE,
    super::TRAILER,
    super::TRANSFER_ENCODING,
    super::UPGRADE,
    super::VARY,
    super::VIA,
    super::WARNING,
    super::WWW_AUTHENTICATE,
    ACCEPT,
];

/// Known single-value fields: duplicates must be identical.
const SINGLE_FIELDS: &[HeaderName] = &[
    super::ACCESS_CONTROL_ALLOW_CREDENTIALS,
    super::ACCESS_CONTROL_ALLOW_ORIGIN,
    super::ACCESS_CONTROL_MAX_AGE,
    super::ACCESS_CONTROL_REQUEST_METHOD,
    super::AGE,
    super::AUTHORIZATION,
    super::CONTENT_DISPOSITION,
    super::CONTENT_LOCATION,
    super::CONTENT_RANGE,
    super::CONTENT_SECURITY_POLICY,
    super::CONTENT_SECURITY_POLICY_REPORT_ONLY,
    super::CONTENT_TYPE,
    super::COOKIE,
    super::DATE,
    super::DNT,
    super::ETAG,
    super::EXPIRES,
    super::FROM,
    super::HOST,
    super::IF_MODIFIED_SINCE,
    super::IF_RANGE,
    super::IF_UNMODIFIED_SINCE,
    super::LAST_MODIFIED,
    super::LOCATION,
    super::MAX_FORWARDS,
    super::ORIGIN,
    super::PROXY_AUTHORIZATION,
    super::PUBLIC_KEY_PINS,
    super::PUBLIC_KEY_PINS_REPORT_ONLY,
    super::RANGE,
    super::REFERER,
    super::REFERRER_POLICY,
    super::REFRESH,
    super::RETRY_AFTER,
    super::SEC_WEBSOCKET_ACCEPT,
    super::SEC_WEBSOCKET_KEY,
    super::SEC_WEBSOCKET_VERSION,
    super::SERVER,
    super::STRICT_TRANSPORT_SECURITY,
    super::UPGRADE_INSECURE_REQUESTS,
    super::USER_AGENT,
    super::X_CONTENT_TYPE_OPTIONS,
    super::X_DNS_PREFETCH_CONTROL,
    super::X_FRAME_OPTIONS,
    super::X_XSS_PROTECTION,
];

/// The fixed set of hop-by-hop headers from [RFC 7230 section 6.1] plus the
/// non-standard `Proxy-Connection` and `Keep-Alive`, which are treated as
/// hop-by-hop in practice.
///
/// [RFC 7230 section 6.1]: https://www.rfc-editor.org/rfc/rfc7230#section-6.1
fn standard_hop_by_hop() -> [HeaderName; 6] {
    [
        HeaderName::from_static("keep-alive"),
        HeaderName::from_static("proxy-connection"),
        super::TE,
        super::TRAILER,
        super::TRANSFER_ENCODING,
        super::UPGRADE,
    ]
}

fn classify(name: &HeaderName) -> Kind {
    if name == SET_COOKIE {
        Kind::MultiKeep
    } else if name == CONTENT_LENGTH {
        Kind::ContentLength
    } else if COMMA_LIST_FIELDS.contains(name) {
        Kind::CommaList
    } else if SINGLE_FIELDS.contains(name) {
        Kind::Single
    } else {
        Kind::UnknownKeep
    }
}

/// Trim leading/trailing OWS (spaces and horizontal tabs).
fn trim_ows(bytes: &[u8]) -> &[u8] {
    fn is_ows(b: &u8) -> bool {
        *b == b' ' || *b == b'\t'
    }
    let bytes = match bytes.iter().position(|b| !is_ows(b)) {
        Some(i) => &bytes[i..],
        None => &[],
    };
    let end = bytes
        .iter()
        .rposition(|b| !is_ows(b))
        .map_or(0, |i| i + 1);
    &bytes[..end]
}

/// Parse a `Content-Length` field value: OWS 1*DIGIT OWS.
fn parse_content_length(value: &HeaderValue) -> Result<u64, ()> {
    let digits = trim_ows(value.as_bytes());
    if digits.is_empty() {
        return Err(());
    }
    let mut n: u64 = 0;
    for &b in digits {
        if !b.is_ascii_digit() {
            return Err(());
        }
        n = n.checked_mul(10).ok_or(())?;
        n = n.checked_add(u64::from(b - b'0')).ok_or(())?;
    }
    Ok(n)
}

/// Split a comma separated field value into trimmed tokens (used to parse
/// `Connection` values).
fn split_comma_tokens(value: &HeaderValue) -> Vec<String> {
    match value.to_str() {
        Ok(s) => s
            .split(',')
            .map(|t| t.trim_matches(|c| c == ' ' || c == '\t'))
            .filter(|t| !t.is_empty())
            .map(str::to_ascii_lowercase)
            .collect(),
        Err(_) => Vec::new(),
    }
}

/// One field worth of validated values, ready to be written to the result.
enum Plan {
    /// Write each value separately.
    Keep(Vec<HeaderValue>),
    /// Write a single combined value.
    One(HeaderValue),
}

impl HeaderMap {
    /// Merge two header maps using the [default policy](MergePolicy::default).
    ///
    /// See [`merge_with`](Self::merge_with) for the exact rules.
    ///
    /// # Examples
    ///
    /// ```
    /// use http::HeaderMap;
    ///
    /// let mut a = HeaderMap::new();
    /// a.insert("accept", "text/html".parse().unwrap());
    ///
    /// let mut b = HeaderMap::new();
    /// b.append("set-cookie", "a=1".parse().unwrap());
    /// b.append("set-cookie", "b=2".parse().unwrap());
    ///
    /// let merged = a.merge(&b).unwrap();
    /// assert_eq!(merged.get_all("set-cookie").iter().count(), 2);
    /// ```
    pub fn merge(&self, other: &HeaderMap) -> Result<HeaderMap, MergeError> {
        self.merge_with(other, &MergePolicy::default())
    }

    /// Merge two header maps, applying field-specific combination rules under
    /// `policy`.
    ///
    /// Neither input map is modified. A new map is returned only when every
    /// field validates; a conflict on any field rejects the whole merge.
    ///
    /// # Rules
    ///
    /// - **List fields** (`Accept`, `Cache-Control`, `Vary`, ...) are
    ///   combined into a single comma separated value.
    /// - **`Set-Cookie`** values are never comma joined: every value becomes
    ///   its own entry.
    /// - **`Content-Length`**: every value in either map must be a valid
    ///   decimal and parse to the same integer.
    /// - **Single-value fields** (`Host`, `ETag`, ...): every value must be
    ///   byte-for-byte identical.
    /// - **Unknown fields** are handled conservatively: every value is kept
    ///   as a separate entry rather than being joined.
    /// - **`Connection`** is merged as a list and, with the default policy,
    ///   every field named by one of its tokens is filtered out along with
    ///   the standard hop-by-hop headers.
    /// - Header names are case insensitive, so different casings never
    ///   produce two keys.
    pub fn merge_with(
        &self,
        other: &HeaderMap,
        policy: &MergePolicy,
    ) -> Result<HeaderMap, MergeError> {
        // Collect the de-duplicated, case-insensitive set of field names.
        // HeaderName already normalizes to lowercase, so iterating both maps
        // with linear membership is enough for header-sized inputs.
        let mut names: Vec<HeaderName> = Vec::new();
        for name in self.keys().chain(other.keys()) {
            if !names.contains(name) {
                names.push(name.clone());
            }
        }

        // Connection tokens name fields that must be filtered.
        let mut connection_tokens: Vec<String> = Vec::new();
        if policy.filter_connection_named {
            for map in [self, other] {
                for value in map.get_all(CONNECTION).iter() {
                    for token in split_comma_tokens(value) {
                        if !connection_tokens.contains(&token) {
                            connection_tokens.push(token);
                        }
                    }
                }
            }
        }

        let hop_by_hop = standard_hop_by_hop();
        let is_filtered = |name: &HeaderName| -> bool {
            // Connection itself is never filtered by its own tokens.
            if name == CONNECTION {
                return false;
            }
            if policy.filter_connection_named
                && connection_tokens
                    .iter()
                    .any(|t| name.as_str() == t.as_str())
            {
                return true;
            }
            if policy.remove_standard_hop_by_hop && hop_by_hop.contains(name) {
                return true;
            }
            false
        };

        // Validate and build a plan per field. Plans are formed before the
        // output map is populated, guaranteeing all-or-nothing semantics and
        // leaving both inputs untouched on failure.
        let mut plans: Vec<(HeaderName, Plan)> = Vec::with_capacity(names.len());

        for name in names {
            if is_filtered(&name) {
                continue;
            }

            let mut values: Vec<&HeaderValue> = Vec::new();
            values.extend(self.get_all(&name).iter());
            values.extend(other.get_all(&name).iter());

            match classify(&name) {
                Kind::MultiKeep | Kind::UnknownKeep => {
                    plans.push((name, Plan::Keep(values.into_iter().cloned().collect())));
                }
                Kind::ContentLength => {
                    let first = parse_content_length(values[0]).map_err(|_| {
                        MergeError::InvalidContentLength {
                            value: values[0].clone(),
                        }
                    })?;
                    for v in &values[1..] {
                        let n = parse_content_length(v).map_err(|_| {
                            MergeError::InvalidContentLength { value: (*v).clone() }
                        })?;
                        if n != first {
                            return Err(MergeError::InvalidContentLength {
                                value: (*v).clone(),
                            });
                        }
                    }
                    plans.push((name, Plan::One(values[0].clone())));
                }
                Kind::Single => {
                    for v in &values[1..] {
                        if *v != values[0] {
                            return Err(MergeError::Conflict { name: name.clone() });
                        }
                    }
                    plans.push((name, Plan::One(values[0].clone())));
                }
                Kind::CommaList => {
                    let mut buf: Vec<u8> = Vec::new();
                    for (i, v) in values.iter().enumerate() {
                        if i > 0 {
                            buf.extend_from_slice(b", ");
                        }
                        buf.extend_from_slice(v.as_bytes());
                    }
                    // The bytes of valid header values joined with ", " are
                    // valid header field content (visible US-ASCII / obs-text).
                    let joined = HeaderValue::from_maybe_shared(bytes::Bytes::from(buf))
                        .expect("joining valid header values is valid");
                    plans.push((name, Plan::One(joined)));
                }
            }
        }

        let mut out = HeaderMap::new();
        for (name, plan) in plans {
            match plan {
                Plan::Keep(values) => {
                    for v in values {
                        out.try_append(name.clone(), v)
                            .map_err(|_| MergeError::TooLarge)?;
                    }
                }
                Plan::One(value) => {
                    out.try_insert(name, value)
                        .map_err(|_| MergeError::TooLarge)?;
                }
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hv(s: &str) -> HeaderValue {
        s.parse().unwrap()
    }

    #[test]
    fn two_set_cookies_remain_two_entries() {
        let mut a = HeaderMap::new();
        a.append(SET_COOKIE, hv("a=1"));
        let mut b = HeaderMap::new();
        b.append(SET_COOKIE, hv("b=2"));

        let merged = a.merge(&b).unwrap();
        let values: Vec<_> = merged.get_all(SET_COOKIE).iter().collect();
        assert_eq!(values, vec![hv("a=1"), hv("b=2")]);

        // Inputs untouched.
        assert_eq!(a.get_all(SET_COOKIE).iter().count(), 1);
        assert_eq!(b.get_all(SET_COOKIE).iter().count(), 1);
    }

    #[test]
    fn list_fields_combine() {
        let mut a = HeaderMap::new();
        a.append(ACCEPT, hv("text/html"));
        let mut b = HeaderMap::new();
        b.append(ACCEPT, hv("application/json"));

        let merged = a.merge(&b).unwrap();
        assert_eq!(merged[ACCEPT], "text/html, application/json");
    }

    #[test]
    fn agreeing_content_length_accepted() {
        let mut a = HeaderMap::new();
        a.insert(CONTENT_LENGTH, hv("42"));
        let mut b = HeaderMap::new();
        b.insert(CONTENT_LENGTH, hv(" 42 "));

        let merged = a.merge(&b).unwrap();
        assert_eq!(merged[CONTENT_LENGTH], "42");
    }

    #[test]
    fn conflicting_content_length_rejected() {
        let mut a = HeaderMap::new();
        a.insert(CONTENT_LENGTH, hv("42"));
        let mut b = HeaderMap::new();
        b.insert(CONTENT_LENGTH, hv("43"));

        assert!(a.merge(&b).is_err());
        // Rejection leaves inputs intact and no partial map escapes.
        assert_eq!(a[CONTENT_LENGTH], "42");
        assert_eq!(b[CONTENT_LENGTH], "43");
    }

    #[test]
    fn invalid_content_length_rejected() {
        let mut a = HeaderMap::new();
        a.insert(CONTENT_LENGTH, hv("42"));
        let mut b = HeaderMap::new();
        b.insert(CONTENT_LENGTH, hv("forty-two"));

        assert!(a.merge(&b).is_err());
    }

    #[test]
    fn name_casing_shares_identity() {
        let mut a = HeaderMap::new();
        a.insert("Content-Type", hv("text/plain"));
        let mut b = HeaderMap::new();
        b.insert("CONTENT-TYPE", hv("text/plain"));

        let merged = a.merge(&b).unwrap();
        assert_eq!(merged.keys_len(), 1);
        assert_eq!(merged["content-type"], "text/plain");
    }

    #[test]
    fn name_casing_conflict_single_field() {
        let mut a = HeaderMap::new();
        a.insert("Content-Type", hv("text/plain"));
        let mut b = HeaderMap::new();
        b.insert("content-type", hv("application/json"));

        let err = a.merge(&b).unwrap_err();
        assert!(matches!(err, MergeError::Conflict { .. }));
    }

    #[test]
    fn connection_named_fields_filtered() {
        let keep_alive = HeaderName::from_static("keep-alive");
        let mut a = HeaderMap::new();
        a.insert(CONNECTION, hv("close, x-foo"));
        a.insert("x-foo", hv("bar"));
        a.insert(&keep_alive, hv("timeout=5"));
        let b = HeaderMap::new();

        let merged = a.merge(&b).unwrap();
        assert!(merged.contains_key(CONNECTION));
        assert!(!merged.contains_key("x-foo"));
        assert!(!merged.contains_key(keep_alive));
    }

    #[test]
    fn retain_all_policy_keeps_named_fields() {
        let mut a = HeaderMap::new();
        a.insert(CONNECTION, hv("x-foo"));
        a.insert("x-foo", hv("bar"));

        let merged = a
            .merge_with(&HeaderMap::new(), &MergePolicy::retain_all())
            .unwrap();
        assert!(merged.contains_key("x-foo"));
    }

    #[test]
    fn unknown_fields_keep_multiple_values() {
        let mut a = HeaderMap::new();
        a.append("x-trace", hv("one"));
        let mut b = HeaderMap::new();
        b.append("x-trace", hv("two"));

        let merged = a.merge(&b).unwrap();
        let values: Vec<_> = merged.get_all("x-trace").iter().collect();
        assert_eq!(values, vec![hv("one"), hv("two")]);
    }

    #[test]
    fn normal_append_is_unchanged() {
        let mut map = HeaderMap::new();
        map.append(SET_COOKIE, hv("a=1"));
        map.append(SET_COOKIE, hv("b=2"));
        assert_eq!(map.get_all(SET_COOKIE).iter().count(), 2);
    }
}
