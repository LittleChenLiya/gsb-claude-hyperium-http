//! Acceptance tests for the explicit `HeaderMap` merge plan.
//!
//! Scoped run: `cargo test --test merge_headers`.

use http::header::{MergeError, MergePolicy, ACCEPT, CONNECTION, CONTENT_LENGTH, SET_COOKIE};
use http::HeaderMap;

fn hv(s: &str) -> http::HeaderValue {
    s.parse().unwrap()
}

#[test]
fn two_set_cookies_are_not_comma_joined() {
    let mut a = HeaderMap::new();
    a.append(SET_COOKIE, hv("a=1"));

    let mut b = HeaderMap::new();
    b.append(SET_COOKIE, hv("b=2; Path=/"));

    let merged = a.merge(&b).unwrap();

    let values: Vec<_> = merged
        .get_all(SET_COOKIE)
        .iter()
        .map(|v| v.to_str().unwrap().to_owned())
        .collect();
    assert_eq!(values, vec!["a=1".to_owned(), "b=2; Path=/".to_owned()]);
    // Exactly one Set-Cookie key, two entries.
    assert_eq!(merged.keys().filter(|k| *k == SET_COOKIE).count(), 1);
    assert_eq!(merged.get_all(SET_COOKIE).iter().count(), 2);
}

#[test]
fn legal_lists_are_combined() {
    let mut a = HeaderMap::new();
    a.insert(ACCEPT, hv("text/html"));
    a.append(ACCEPT, hv("text/plain"));

    let mut b = HeaderMap::new();
    b.insert(ACCEPT, hv("application/json"));

    let merged = a.merge(&b).unwrap();
    assert_eq!(
        merged[ACCEPT],
        "text/html, text/plain, application/json"
    );
}

#[test]
fn conflicting_content_length_is_rejected() {
    let mut a = HeaderMap::new();
    a.insert(CONTENT_LENGTH, hv("42"));

    let mut b = HeaderMap::new();
    b.insert(CONTENT_LENGTH, hv("43"));

    match a.merge(&b) {
        Err(MergeError::InvalidContentLength { .. }) => {}
        other => panic!("expected InvalidContentLength, got {:?}", other),
    }
}

#[test]
fn different_name_casings_do_not_make_two_keys() {
    let mut a = HeaderMap::new();
    a.insert("Content-Length", hv("7"));

    let mut b = HeaderMap::new();
    b.insert("CONTENT-length", hv("7"));

    let merged = a.merge(&b).unwrap();
    assert_eq!(merged.keys_len(), 1);
    assert_eq!(merged[CONTENT_LENGTH], "7");
}

#[test]
fn unknown_field_conservatively_keeps_multiple_values() {
    let mut a = HeaderMap::new();
    a.append("x-request-id", hv("aaa"));

    let mut b = HeaderMap::new();
    b.append("X-REQUEST-ID", hv("bbb"));

    let merged = a.merge(&b).unwrap();
    assert_eq!(merged.keys_len(), 1);
    let values: Vec<_> = merged
        .get_all("x-request-id")
        .iter()
        .map(|v| v.to_str().unwrap().to_owned())
        .collect();
    assert_eq!(values, vec!["aaa".to_owned(), "bbb".to_owned()]);
}

#[test]
fn connection_named_fields_are_filtered() {
    let mut a = HeaderMap::new();
    a.insert(CONNECTION, hv("keep-alive"));
    a.insert("keep-alive", hv("timeout=5, max=1000"));

    let b = HeaderMap::new();
    let merged = a.merge(&b).unwrap();

    assert!(merged.contains_key(CONNECTION));
    assert!(!merged.contains_key("keep-alive"));
}

#[test]
fn inputs_are_never_modified_on_success_or_failure() {
    let mut a = HeaderMap::new();
    a.append(SET_COOKIE, hv("a=1"));
    a.insert(CONTENT_LENGTH, hv("1"));

    let mut b = HeaderMap::new();
    b.append(SET_COOKIE, hv("b=2"));
    b.insert(CONTENT_LENGTH, hv("2"));

    assert!(a.merge(&b).is_err());

    assert_eq!(a.get_all(SET_COOKIE).iter().count(), 1);
    assert_eq!(b.get_all(SET_COOKIE).iter().count(), 1);
    assert_eq!(a[CONTENT_LENGTH], "1");
    assert_eq!(b[CONTENT_LENGTH], "2");
}

#[test]
fn ordinary_append_is_unaffected() {
    let mut map = HeaderMap::new();
    map.append(SET_COOKIE, hv("a=1"));
    map.append(SET_COOKIE, hv("b=2"));
    map.append(ACCEPT, hv("text/html"));
    map.append(ACCEPT, hv("application/json"));

    assert_eq!(map.get_all(SET_COOKIE).iter().count(), 2);
    // Plain append does not comma-join anything.
    let accept: Vec<_> = map.get_all(ACCEPT).iter().collect();
    assert_eq!(accept, vec![hv("text/html"), hv("application/json")]);
}

#[test]
fn retain_all_policy_skips_connection_filtering() {
    let mut a = HeaderMap::new();
    a.insert(CONNECTION, hv("x-hop"));
    a.insert("x-hop", hv("v"));

    let merged = a
        .merge_with(&HeaderMap::new(), &MergePolicy::retain_all())
        .unwrap();
    assert!(merged.contains_key("x-hop"));
}

#[test]
fn five_header_example_shape() {
    let mut a = HeaderMap::new();
    a.append(SET_COOKIE, hv("session=abc"));
    a.insert(ACCEPT, hv("text/html"));
    a.insert(CONTENT_LENGTH, hv("42"));
    a.insert(CONNECTION, hv("keep-alive"));
    a.append("x-custom", hv("one"));

    let mut b = HeaderMap::new();
    b.append(SET_COOKIE, hv("tracking=xyz"));
    b.insert(ACCEPT, hv("application/json"));
    b.insert(CONTENT_LENGTH, hv("42"));
    b.insert("keep-alive", hv("timeout=5"));
    b.append("x-custom", hv("two"));

    let merged = a.merge(&b).unwrap();

    assert_eq!(merged.get_all(SET_COOKIE).iter().count(), 2);
    assert_eq!(merged[ACCEPT], "text/html, application/json");
    assert_eq!(merged[CONTENT_LENGTH], "42");
    assert_eq!(merged[CONNECTION], "keep-alive");
    assert!(!merged.contains_key("keep-alive"));
    assert_eq!(merged.get_all("x-custom").iter().count(), 2);
}

