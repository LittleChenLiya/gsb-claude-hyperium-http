//! Acceptance tests for the explicit `HeaderMap` merge plan.
//!
//! Five headers exercise every branch: `accept` (list), `content-length`
//! (single-value agreement), `set-cookie` (non-mergeable multi-value),
//! `connection` plus the field it names (connection-specific filtering), and
//! `x-custom` (an unknown field handled conservatively).

use http::header::{
    merge, merge_with_options, ConnectionFields, MergeError, MergeOptions, ACCEPT, CONNECTION,
    CONTENT_LENGTH, CONTENT_TYPE, HOST, SET_COOKIE, VARY,
};
use http::HeaderName;
use http::{HeaderMap, HeaderValue};

fn v(s: &str) -> HeaderValue {
    s.parse().unwrap()
}

fn name(s: &str) -> HeaderName {
    s.parse().unwrap()
}

fn dump(map: &HeaderMap) -> Vec<(String, String)> {
    let mut out: Vec<_> = map
        .iter()
        .map(|(k, val)| (k.as_str().to_string(), val.to_str().unwrap().to_string()))
        .collect();
    out.sort();
    out
}

#[test]
fn set_cookie_keeps_two_separate_entries() {
    let mut a = HeaderMap::new();
    a.append(SET_COOKIE, v("a=1; Path=/"));
    let mut b = HeaderMap::new();
    b.append(SET_COOKIE, v("b=2; Path=/"));

    let merged = merge(&a, &b).unwrap();

    let cookies: Vec<_> = merged.get_all(SET_COOKIE).iter().cloned().collect();
    assert_eq!(cookies.len(), 2, "two Set-Cookie values must survive");
    assert_eq!(cookies[0], "a=1; Path=/");
    assert_eq!(cookies[1], "b=2; Path=/");
    // They must not collapse into one comma-joined value.
    for cookie in &cookies {
        assert!(!cookie.to_str().unwrap().contains(","));
    }
}

#[test]
fn known_list_headers_merge_with_comma() {
    let mut a = HeaderMap::new();
    a.append(ACCEPT, v("text/html"));
    a.append(VARY, v("accept-encoding"));
    let mut b = HeaderMap::new();
    b.append(ACCEPT, v("application/json"));
    b.append(VARY, v("origin"));

    let merged = merge(&a, &b).unwrap();

    assert_eq!(merged[ACCEPT], "text/html, application/json");
    assert_eq!(merged[VARY], "accept-encoding, origin");
    assert_eq!(merged.get_all(ACCEPT).iter().count(), 1);
    assert_eq!(merged.get_all(VARY).iter().count(), 1);
}

#[test]
fn conflicting_content_length_is_rejected() {
    let mut a = HeaderMap::new();
    a.insert(CONTENT_LENGTH, v("42"));
    let mut b = HeaderMap::new();
    b.insert(CONTENT_LENGTH, v("43"));

    let err = merge(&a, &b).expect_err("different lengths must be rejected");
    match err {
        MergeError::ContentLengthConflict { first, other } => {
            assert_eq!(first, "42");
            assert_eq!(other, "43");
        }
        other => panic!("wrong error: {other:?}"),
    }

    // Failure leaves the inputs intact.
    assert_eq!(a[CONTENT_LENGTH], "42");
    assert_eq!(b[CONTENT_LENGTH], "43");
}

#[test]
fn equal_content_length_collapses_to_one() {
    let mut a = HeaderMap::new();
    a.insert(CONTENT_LENGTH, v("42"));
    let mut b = HeaderMap::new();
    b.insert(name("Content-Length"), v("42"));

    let merged = merge(&a, &b).unwrap();
    assert_eq!(merged[CONTENT_LENGTH], "42");
    assert_eq!(merged.get_all(CONTENT_LENGTH).iter().count(), 1);
}

#[test]
fn header_name_case_shares_one_identity() {
    // Different spellings of one standard name collapse to one key.
    let mut a = HeaderMap::new();
    a.insert(name("Content-Length"), v("7"));
    let mut b = HeaderMap::new();
    b.insert(name("CONTENT-LENGTH"), v("7"));

    let merged = merge(&a, &b).unwrap();
    assert_eq!(merged.keys_len(), 1);
    assert_eq!(merged[CONTENT_LENGTH], "7");

    // Same for an unknown custom name: no duplicated key from casing.
    let mut a = HeaderMap::new();
    a.append(name("X-Custom-Thing"), v("one"));
    let mut b = HeaderMap::new();
    b.append(name("x-custom-thing"), v("two"));

    let merged = merge(&a, &b).unwrap();
    assert_eq!(merged.keys_len(), 1);
    let values: Vec<_> = merged
        .get_all("x-custom-thing")
        .iter()
        .map(|x| x.to_str().unwrap().to_string())
        .collect();
    assert_eq!(values, vec!["one", "two"]);
}

#[test]
fn unknown_custom_header_conservatively_keeps_multiple_values() {
    let mut a = HeaderMap::new();
    a.append(name("x-unknown-listish"), v("a"));
    a.append(name("x-unknown-listish"), v("b"));
    let mut b = HeaderMap::new();
    b.append(name("x-unknown-listish"), v("c"));

    let merged = merge(&a, &b).unwrap();

    let values: Vec<_> = merged
        .get_all("x-unknown-listish")
        .iter()
        .map(|x| x.to_str().unwrap().to_string())
        .collect();
    // Values are retained verbatim: neither comma-joined nor rejected.
    assert_eq!(values, vec!["a", "b", "c"]);
}

#[test]
fn connection_named_fields_are_filtered_by_default() {
    let mut a = HeaderMap::new();
    a.append(CONNECTION, v("keep-alive, x-foo"));
    a.insert(name("keep-alive"), v("timeout=5"));
    a.append(name("x-foo"), v("bar"));
    // A field not named by connection survives.
    a.insert(HOST, v("example.com"));
    let mut b = HeaderMap::new();
    b.append(name("x-foo"), v("baz"));

    let merged = merge(&a, &b).unwrap();

    assert!(merged.get(CONNECTION).is_none(), "connection is removed");
    assert!(
        merged.get("keep-alive").is_none(),
        "a field named by connection is removed"
    );
    assert!(merged.get("x-foo").is_none());
    assert_eq!(merged[HOST], "example.com");
}

#[test]
fn connection_fields_can_be_kept_explicitly() {
    let mut a = HeaderMap::new();
    a.append(CONNECTION, v("keep-alive"));
    a.insert(name("keep-alive"), v("timeout=5"));
    let b = HeaderMap::new();

    let opts = MergeOptions {
        connection_fields: ConnectionFields::Keep,
    };
    let merged = merge_with_options(&a, &b, &opts).unwrap();

    assert_eq!(merged[CONNECTION], "keep-alive");
    assert_eq!(merged["keep-alive"], "timeout=5");
}

#[test]
fn single_value_conflict_is_rejected_with_named_error() {
    let mut a = HeaderMap::new();
    a.insert(HOST, v("a.example"));
    let mut b = HeaderMap::new();
    b.insert(HOST, v("b.example"));

    match merge(&a, &b) {
        Err(MergeError::SingletonConflict {
            name: n,
            first,
            other,
        }) => {
            assert_eq!(n, HOST);
            assert_eq!(first, "a.example");
            assert_eq!(other, "b.example");
        }
        other => panic!("expected singleton conflict, got {other:?}"),
    }
}

#[test]
fn merge_returns_new_map_and_never_touches_inputs() {
    let mut a = HeaderMap::new();
    a.insert(ACCEPT, v("text/html"));
    a.insert(CONTENT_TYPE, v("text/plain"));
    a.append(SET_COOKIE, v("a=1"));
    let mut b = HeaderMap::new();
    b.insert(ACCEPT, v("*/*"));
    b.insert(CONTENT_TYPE, v("text/plain"));
    b.append(SET_COOKIE, v("b=2"));

    let snapshot_a = dump(&a);
    let snapshot_b = dump(&b);

    let merged = merge(&a, &b).unwrap();

    assert_eq!(dump(&a), snapshot_a);
    assert_eq!(dump(&b), snapshot_b);
    assert_ne!(merged.keys_len(), 0);
    assert_eq!(a[ACCEPT], "text/html");
    assert_eq!(b[ACCEPT], "*/*");
}

#[test]
fn five_headers_end_to_end() {
    // Five headers exercising every branch of the plan.
    let mut a = HeaderMap::new();
    a.insert(ACCEPT, v("text/html"));
    a.insert(CONTENT_LENGTH, v("10"));
    a.append(SET_COOKIE, v("a=1"));
    a.append(CONNECTION, v("keep-alive"));
    a.insert(name("keep-alive"), v("timeout=5"));
    a.append(name("x-custom"), v("p=1"));

    let mut b = HeaderMap::new();
    b.insert(ACCEPT, v("application/json"));
    b.insert(name("Content-Length"), v("10"));
    b.append(SET_COOKIE, v("b=2"));
    b.append(name("x-custom"), v("q=2"));

    let merged = merge(&a, &b).unwrap();

    assert_eq!(merged[ACCEPT], "text/html, application/json");
    assert_eq!(merged[CONTENT_LENGTH], "10");
    assert_eq!(merged.get_all(SET_COOKIE).iter().count(), 2);
    assert!(merged.get(CONNECTION).is_none());
    assert!(merged.get("keep-alive").is_none());
    assert_eq!(
        merged
            .get_all("x-custom")
            .iter()
            .map(|x| x.to_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["p=1", "q=2"]
    );
}

#[test]
fn ordinary_append_is_unchanged() {
    let mut map = HeaderMap::new();
    assert!(!map.append(HOST, v("example.com")));
    assert!(map.append(HOST, v("other.example")));

    let values: Vec<_> = map.get_all(HOST).iter().cloned().collect();
    assert_eq!(values, vec![v("example.com"), v("other.example")]);

    map.append(SET_COOKIE, v("a=1"));
    map.append(SET_COOKIE, v("b=2"));
    assert_eq!(map.get_all(SET_COOKIE).iter().count(), 2);
}
