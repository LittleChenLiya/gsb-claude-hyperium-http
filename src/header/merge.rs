//! Explicit merging of two [`HeaderMap`]s.
//!
//! Unlike [`HeaderMap::append`], which unconditionally pushes a value onto the
//! end of the value list for a name, [`merge`] follows an explicit, per-field
//! merge plan derived from how each header is defined:
//!
//! * **List headers** (such as `accept` or `vary`) are comma-separated value
//!   lists. Every value from both inputs is combined into a single
//!   comma-separated value, preserving relative order. `set-cookie` is
//!   deliberately **not** handled this way.
//! * **Single-value headers** (such as `host` or `content-type`) may only
//!   agree across the inputs: repeated equal values collapse to one, while
//!   differing values reject the merge.
//! * **Non-mergeable multi-value headers** (`set-cookie`) keep every value as
//!   a separate entry; they are never joined with commas.
//! * **`content-length`** uses the same agreement rule as single-value
//!   headers, but reports a dedicated
//!   [`ContentLengthConflict`](MergeError::ContentLengthConflict) error so
//!   callers can spot the classic duplicate-length problem.
//! * **Unknown (custom) headers** are handled conservatively: all of their
//!   values are retained as separate entries, since the semantics of an
//!   unrecognized field cannot be guessed.
//!
//! Headers named by a `connection` value are connection-specific per
//! [RFC 9110 Section 7.6.1] and are, by default, filtered out of the result
//! together with the `connection` header itself. See [`ConnectionFields`] for
//! opting out.
//!
//! Header names share one identity regardless of case: `Content-Length`,
//! `content-length` and `CONTENT-LENGTH` are the same key, so casing variants
//! never produce two keys.
//!
//! Merging is all-or-nothing: validation runs first and, on success, a brand
//! new `HeaderMap` is returned. Neither input is modified, even on failure.
//! `HeaderMap::append` itself is untouched by this module.
//!
//! [RFC 9110 Section 7.6.1]: https://www.rfc-editor.org/rfc/rfc9110#section-7.6.1

use std::collections::HashMap;
use std::error::Error;
use std::fmt;

use super::name::HeaderName;
use super::value::HeaderValue;
use super::{
    HeaderMap, ACCEPT, ACCEPT_CHARSET, ACCEPT_ENCODING, ACCEPT_LANGUAGE, ACCEPT_RANGES, ALLOW,
    CACHE_CONTROL, CONNECTION, CONTENT_ENCODING, CONTENT_LANGUAGE, CONTENT_LENGTH, EXPECT,
    FORWARDED, IF_MATCH, IF_NONE_MATCH, PRAGMA, PROXY_AUTHENTICATE, PROXY_AUTHORIZATION,
    SET_COOKIE, TE, TRAILER, TRANSFER_ENCODING, UPGRADE, VARY, VIA, WARNING, WWW_AUTHENTICATE,
};

/// Policy for fields named by the `connection` header.
///
/// RFC 9110 section 7.6.1 says that `connection` lists control options and
/// field names intended to be specific to a single connection. Such fields
/// (and `connection` itself) must not be forwarded when a message is proxied
/// or merged across connections.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ConnectionFields {
    /// Drop every header whose name appears as a token in any `connection`
    /// value, and drop the `connection` header itself. This is the default.
    Filter,

    /// Keep the `connection` header (merged as a list) and every field it
    /// names. Use this only when the merged message stays on the same
    /// connection.
    Keep,
}

impl Default for ConnectionFields {
    fn default() -> Self {
        ConnectionFields::Filter
    }
}

/// Options controlling [`merge_with_options`].
///
/// Use [`MergeOptions::default`] for the recommended behavior, or construct
/// and tweak individual fields:
///
/// ```
/// use http::header::{merge_with_options, ConnectionFields, HeaderMap, MergeOptions};
///
/// let a = HeaderMap::new();
/// let b = HeaderMap::new();
/// let opts = MergeOptions {
///     connection_fields: ConnectionFields::Keep,
/// };
/// let merged = merge_with_options(&a, &b, &opts).unwrap();
/// assert!(merged.is_empty());
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MergeOptions {
    /// Whether headers named by `connection`, and `connection` itself, are
    /// filtered out of the result.
    pub connection_fields: ConnectionFields,
}

impl Default for MergeOptions {
    fn default() -> Self {
        MergeOptions {
            connection_fields: ConnectionFields::Filter,
        }
    }
}

/// An error returned when two [`HeaderMap`]s cannot be merged consistently.
///
/// A merge is all-or-nothing: when any variant is returned, no partial result
/// is produced and both inputs remain untouched.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum MergeError {
    /// Both inputs define `content-length` and the lengths differ.
    ContentLengthConflict {
        /// The first value seen for `content-length` (trimmed of OWS).
        first: HeaderValue,
        /// The later, non-matching value.
        other: HeaderValue,
    },

    /// A single-value header appears with differing values.
    SingletonConflict {
        /// The conflicting header name.
        name: HeaderName,
        /// The first value seen for the header.
        first: HeaderValue,
        /// The later, non-matching value.
        other: HeaderValue,
    },

    /// The merged headers do not fit in a `HeaderMap` (capacity reached).
    MaxSizeReached,
}

impl fmt::Display for MergeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MergeError::ContentLengthConflict { first, other } => write!(
                f,
                "conflicting content-length headers: {:?} vs {:?}",
                first, other
            ),
            MergeError::SingletonConflict { name, first, other } => write!(
                f,
                "conflicting values for single-value header {:?}: {:?} vs {:?}",
                name, first, other
            ),
            MergeError::MaxSizeReached => {
                f.write_str("merged HeaderMap would exceed its maximum size")
            }
        }
    }
}

impl Error for MergeError {}

/// How values for a header name are merged across the inputs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FieldClass {
    /// Comma-separated list; all values join into one value.
    List,
    /// Exactly one value; differing inputs conflict.
    Singleton,
    /// Multiple values that must stay separate, never comma-joined.
    Multi,
    /// `content-length`; singleton rule with a dedicated error.
    ContentLength,
    /// `connection`; a list that is dropped when filtering is selected.
    Connection,
}

#[derive(Debug, Clone, Copy)]
struct Classification {
    class: FieldClass,
}

impl FieldClass {
    fn classify(name: &HeaderName) -> Classification {
        let class = if name == SET_COOKIE {
            FieldClass::Multi
        } else if name == CONTENT_LENGTH {
            FieldClass::ContentLength
        } else if name == CONNECTION {
            FieldClass::Connection
        } else if is_known_list(name) {
            FieldClass::List
        } else if name.is_standard() {
            // A standard header that is neither a list nor set-cookie:
            // content-type, host, location, date, ... carry one value.
            FieldClass::Singleton
        } else {
            // Unrecognized custom header: never guess, keep every value.
            FieldClass::Multi
        };

        Classification { class }
    }
}

/// Header names whose value grammar is a comma-separated list (`#element`).
const KNOWN_LISTS: &[HeaderName] = &[
    ACCEPT,
    ACCEPT_CHARSET,
    ACCEPT_ENCODING,
    ACCEPT_LANGUAGE,
    ACCEPT_RANGES,
    ALLOW,
    CACHE_CONTROL,
    CONTENT_ENCODING,
    CONTENT_LANGUAGE,
    EXPECT,
    FORWARDED,
    IF_MATCH,
    IF_NONE_MATCH,
    PRAGMA,
    PROXY_AUTHENTICATE,
    PROXY_AUTHORIZATION,
    TE,
    TRAILER,
    TRANSFER_ENCODING,
    UPGRADE,
    VARY,
    VIA,
    WARNING,
    WWW_AUTHENTICATE,
];

fn is_known_list(name: &HeaderName) -> bool {
    KNOWN_LISTS.contains(name)
}

/// Merge two header maps using the default [`MergeOptions`].
///
/// See the [module documentation](self) for the per-field merge plan.
///
/// # Examples
///
/// ```
/// use http::header::{
///     merge, ACCEPT, CONNECTION, CONTENT_LENGTH, SET_COOKIE,
/// };
/// use http::HeaderMap;
///
/// let mut a = HeaderMap::new();
/// a.insert(ACCEPT, "text/html".parse().unwrap());
/// a.insert(CONTENT_LENGTH, "42".parse().unwrap());
/// a.append(SET_COOKIE, "a=1".parse().unwrap());
/// a.append(CONNECTION, "keep-alive".parse().unwrap());
///
/// let mut b = HeaderMap::new();
/// b.insert(ACCEPT, "application/json".parse().unwrap());
/// b.append(SET_COOKIE, "b=2".parse().unwrap());
///
/// let merged = merge(&a, &b).unwrap();
///
/// // List headers become one comma-separated value.
/// assert_eq!(merged[ACCEPT], "text/html, application/json");
/// // Set-Cookie keeps two entries instead of one comma-joined value.
/// assert_eq!(merged.get_all(SET_COOKIE).iter().count(), 2);
/// // Connection-named ("keep-alive") fields, and connection itself, are gone.
/// assert!(merged.get(CONNECTION).is_none());
/// ```
pub fn merge(a: &HeaderMap, b: &HeaderMap) -> Result<HeaderMap, MergeError> {
    merge_with_options(a, b, &MergeOptions::default())
}

/// Merge two header maps with explicit [`MergeOptions`].
///
/// Neither input is modified; on success a fresh `HeaderMap` is returned.
pub fn merge_with_options(
    a: &HeaderMap,
    b: &HeaderMap,
    options: &MergeOptions,
) -> Result<HeaderMap, MergeError> {
    // 1. Collect the header names declared connection-specific. This happens
    //    before any validation so filtering and merging use one consistent set.
    let mut connection_named: Vec<HeaderName> = Vec::new();
    if options.connection_fields == ConnectionFields::Filter {
        for map in [a, b] {
            for value in map.get_all(CONNECTION).iter() {
                collect_connection_tokens(value, &mut connection_named);
            }
        }
    }

    // 2. Aggregate values according to the field classification. Values are
    //    cloned: the inputs are only borrowed and must stay untouched.
    let mut plan: HashMap<HeaderName, Vec<HeaderValue>> = HashMap::new();
    let mut order: Vec<HeaderName> = Vec::new();

    for map in [a, b] {
        for (name, value) in map.iter() {
            if options.connection_fields == ConnectionFields::Filter {
                if name == CONNECTION || connection_named.iter().any(|n| n == name) {
                    continue;
                }
            }

            let Classification { class } = FieldClass::classify(name);

            let values = plan.entry(name.clone()).or_insert_with(|| {
                order.push(name.clone());
                Vec::new()
            });

            match class {
                FieldClass::List | FieldClass::Connection => {
                    // Accumulate raw values; joining happens after all inputs
                    // are seen, so input bytes are never mutated in place.
                    values.push(value.clone());
                }
                FieldClass::Multi => {
                    values.push(value.clone());
                }
                FieldClass::Singleton => {
                    if let Some(existing) = values.first() {
                        if !values_equal(existing, value) {
                            return Err(MergeError::SingletonConflict {
                                name: name.clone(),
                                first: existing.clone(),
                                other: value.clone(),
                            });
                        }
                    } else {
                        values.push(value.clone());
                    }
                }
                FieldClass::ContentLength => {
                    if let Some(existing) = values.first() {
                        if !lengths_equal(existing, value) {
                            return Err(MergeError::ContentLengthConflict {
                                first: existing.clone(),
                                other: value.clone(),
                            });
                        }
                    } else {
                        values.push(value.clone());
                    }
                }
            }
        }
    }

    // 3. Build the output only after every check has passed.
    let mut out = HeaderMap::new();
    for name in &order {
        let Classification { class } = FieldClass::classify(name);
        let values = plan.remove(name).expect("every ordered name has values");

        match class {
            FieldClass::List | FieldClass::Connection => {
                if let Some(joined) = join_comma_list(&values) {
                    out.try_insert(name.clone(), joined)
                        .map_err(|_| MergeError::MaxSizeReached)?;
                }
                // A header with only empty values contributes nothing.
            }
            FieldClass::Multi | FieldClass::Singleton | FieldClass::ContentLength => {
                for value in values {
                    out.try_append(name.clone(), value)
                        .map_err(|_| MergeError::MaxSizeReached)?;
                }
            }
        }
    }

    Ok(out)
}

/// Split a `connection` value into the field-name tokens it declares.
///
/// Tokens are matched case-insensitively by parsing them as `HeaderName`s
/// (which normalizes case); malformed tokens are ignored conservatively.
fn collect_connection_tokens(value: &HeaderValue, out: &mut Vec<HeaderName>) {
    if let Ok(text) = value.to_str() {
        for raw in text.split(',') {
            let token = raw.trim();
            if token.is_empty() {
                continue;
            }
            if let Ok(named) = HeaderName::from_bytes(token.as_bytes()) {
                if !out.iter().any(|n| n == &named) {
                    out.push(named);
                }
            }
        }
    }
}

/// Compare single-value header bytes after trimming OWS; skip trimming when
/// the raw bytes already match.
fn values_equal(a: &HeaderValue, b: &HeaderValue) -> bool {
    a.as_bytes() == b.as_bytes() || trim_ows(a.as_bytes()) == trim_ows(b.as_bytes())
}

/// Compare two `content-length` values.
///
/// Surrounding OWS is ignored; two lengths must agree byte-for-byte on the
/// trimmed digits so malformed values never silently merge.
fn lengths_equal(a: &HeaderValue, b: &HeaderValue) -> bool {
    let a = trim_ows(a.as_bytes());
    let b = trim_ows(b.as_bytes());
    a == b
}

fn trim_ows(bytes: &[u8]) -> &[u8] {
    let mut s = bytes;
    while matches!(s.first(), Some(b' ') | Some(b'\t')) {
        s = &s[1..];
    }
    while matches!(s.last(), Some(b' ') | Some(b'\t')) {
        s = &s[..s.len() - 1];
    }
    s
}

/// Join accumulated list-header values with `", "`, skipping empty fragments.
/// Returns `None` when no non-empty fragment exists.
fn join_comma_list(values: &[HeaderValue]) -> Option<HeaderValue> {
    let mut total = 0usize;
    let mut non_empty = 0usize;
    for value in values {
        let bytes = trim_ows(value.as_bytes());
        if !bytes.is_empty() {
            total = total.saturating_add(bytes.len());
            non_empty += 1;
        }
    }

    if non_empty == 0 {
        return None;
    }

    let mut buf = Vec::with_capacity(total + 2 * (non_empty - 1));
    for value in values {
        let bytes = trim_ows(value.as_bytes());
        if bytes.is_empty() {
            continue;
        }
        if !buf.is_empty() {
            buf.extend_from_slice(b", ");
        }
        buf.extend_from_slice(bytes);
    }

    // Every fragment was already a valid HeaderValue, and trimming removed
    // only leading/trailing OWS; joining with ", " preserves validity.
    Some(HeaderValue::from_bytes(&buf).expect("joining valid values with ', ' stays valid"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::header::{HeaderMap, HOST, SET_COOKIE};

    fn hv(s: &str) -> HeaderValue {
        s.parse().unwrap()
    }

    #[test]
    fn classification_rules() {
        assert_eq!(FieldClass::classify(&SET_COOKIE).class, FieldClass::Multi);
        assert_eq!(
            FieldClass::classify(&CONTENT_LENGTH).class,
            FieldClass::ContentLength
        );
        assert_eq!(
            FieldClass::classify(&CONNECTION).class,
            FieldClass::Connection
        );
        assert_eq!(FieldClass::classify(&VARY).class, FieldClass::List);
        assert_eq!(FieldClass::classify(&HOST).class, FieldClass::Singleton);
        let custom = HeaderName::from_static("x-something");
        assert!(!custom.is_standard());
        assert_eq!(FieldClass::classify(&custom).class, FieldClass::Multi);
    }

    #[test]
    fn same_map_duplicate_content_length_must_agree() {
        // Two values inside ONE map still count as multiple Content-Length.
        let mut a = HeaderMap::new();
        // insert keeps the head; append pushes a second value
        a.insert(CONTENT_LENGTH, hv("5"));
        a.append(CONTENT_LENGTH, hv("5"));
        let b = HeaderMap::new();

        let merged = merge(&a, &b).unwrap();
        assert_eq!(merged.get_all(CONTENT_LENGTH).iter().count(), 1);

        let mut a = HeaderMap::new();
        a.append(CONTENT_LENGTH, hv("5"));
        a.append(CONTENT_LENGTH, hv("6"));
        assert!(matches!(
            merge(&a, &b),
            Err(MergeError::ContentLengthConflict { .. })
        ));
    }

    #[test]
    fn connection_tokens_are_case_insensitive_and_deduped() {
        let mut tokens = Vec::new();
        collect_connection_tokens(&hv("Keep-Alive, X-FOO , x-foo"), &mut tokens);
        assert_eq!(
            tokens,
            vec![
                HeaderName::from_static("keep-alive"),
                HeaderName::from_static("x-foo"),
            ]
        );
    }

    #[test]
    fn list_join_skips_empty_and_trims_ows() {
        let joined = join_comma_list(&[hv("  a  "), hv(""), hv("b")])
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();
        assert_eq!(joined, "a, b");

        assert!(join_comma_list(&[hv(" ")]).is_none());
    }

    #[test]
    fn singleton_ows_differences_are_equivalent() {
        let mut a = HeaderMap::new();
        a.insert(HOST, hv("example.com"));
        let mut b = HeaderMap::new();
        b.insert(HOST, hv(" example.com "));
        assert!(merge(&a, &b).is_ok());
    }
}
