//! Acceptance tests for the opt-in, reversible `HeaderMap::remove_all` API.
//!
//! Run with:
//!     cargo test --test header_map_remove_all --features remove_all
//!
//! Without the feature the new API must not exist, while the legacy `remove`
//! interface keeps working:
//!     cargo test --test header_map_remove_all

#[cfg(feature = "remove_all")]
use http::header::HeaderName;
use http::header::{HeaderValue, COOKIE, SET_COOKIE};
use http::HeaderMap;

/// In-memory fixture: repeated cookies, a differently-cased lookup for the
/// same header name, and a sensitive value mixed in.
fn build_fixture() -> HeaderMap {
    let mut map = HeaderMap::new();

    map.append(SET_COOKIE, HeaderValue::from_static("sid=abc; Path=/"));

    map.append(COOKIE, HeaderValue::from_static("a=1"));
    map.append(COOKIE, HeaderValue::from_static("b=2"));
    let mut secret = HeaderValue::from_static("session=topsecret");
    secret.set_sensitive(true);
    map.append(COOKIE, secret);
    map.append(COOKIE, HeaderValue::from_static("c=3"));

    map.append(SET_COOKIE, HeaderValue::from_static("theme=dark; Path=/"));

    map
}

#[cfg(feature = "remove_all")]
fn original_cookie_order() -> [&'static str; 4] {
    ["a=1", "b=2", "session=topsecret", "c=3"]
}

#[cfg(feature = "remove_all")]
#[test]
fn missing_key_returns_empty_vec_and_changes_nothing() {
    let mut map = build_fixture();
    let before: Vec<_> = map.iter().map(|(k, v)| (k.clone(), v.clone())).collect();

    let removed: Vec<HeaderValue> = map.remove_all("x-not-present");
    assert!(removed.is_empty(), "missing key must yield an empty vec");

    let after: Vec<_> = map.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
    assert_eq!(
        before, after,
        "map must be untouched when the key is absent"
    );
}

#[cfg(feature = "remove_all")]
#[test]
fn removes_all_repeated_values_in_original_order_preserving_sensitive() {
    let mut map = build_fixture();

    // Different casing resolves to the same `HeaderName` identity.
    let removed = map.remove_all("CoOkIe");

    let expected = original_cookie_order();
    assert_eq!(removed.len(), expected.len());
    for (value, raw) in removed.iter().zip(expected.iter()) {
        assert_eq!(value, *raw);
    }

    // Explicit removal order: first inserted value comes first, and the
    // sensitive bit travels with its value.
    assert_eq!(removed[0], "a=1");
    assert!(!removed[0].is_sensitive());
    assert_eq!(removed[1], "b=2");
    assert!(!removed[1].is_sensitive());
    assert_eq!(removed[2], "session=topsecret");
    assert!(
        removed[2].is_sensitive(),
        "removed value must keep its sensitive flag"
    );
    assert_eq!(removed[3], "c=3");
    assert!(!removed[3].is_sensitive());

    // The cookie entry and every one of its values is gone.
    assert!(map.get_all(COOKIE).iter().next().is_none());
    assert!(map.get("cookie").is_none());
    assert!(!map.contains_key(COOKIE));

    // Headers stored under other names survive untouched, in their own order.
    let set_cookies: Vec<_> = map.get_all(SET_COOKIE).iter().collect();
    assert_eq!(set_cookies.len(), 2);
    assert_eq!(set_cookies[0], "sid=abc; Path=/");
    assert_eq!(set_cookies[1], "theme=dark; Path=/");

    // Remaining map only carries the two SET_COOKIE values.
    assert_eq!(map.len(), 2);
    assert_eq!(map.keys_len(), 1);
}

#[cfg(feature = "remove_all")]
#[test]
fn removed_values_can_be_inserted_back_to_restore_original_state() {
    let mut map = build_fixture();

    let snapshot: Vec<(HeaderName, HeaderValue)> =
        map.iter().map(|(k, v)| (k.clone(), v.clone())).collect();

    let key = HeaderName::from_static("cookie");
    let removed = map.remove_all(key.clone());

    assert!(map.get_all(&key).iter().next().is_none());

    // Re-insert in the returned order.
    for value in removed {
        map.append(key.clone(), value);
    }

    let restored: Vec<(HeaderName, HeaderValue)> =
        map.iter().map(|(k, v)| (k.clone(), v.clone())).collect();

    assert_eq!(
        snapshot, restored,
        "reinsertion must restore the original map"
    );

    // Sensitive flag survived the round trip on its original position.
    let cookies: Vec<_> = map.get_all(COOKIE).iter().cloned().collect();
    let expected = original_cookie_order();
    assert_eq!(cookies.len(), expected.len());
    for (value, raw) in cookies.iter().zip(expected.iter()) {
        assert_eq!(value, *raw);
    }
    assert!(!cookies[0].is_sensitive());
    assert!(!cookies[1].is_sensitive());
    assert!(cookies[2].is_sensitive());
    assert!(!cookies[3].is_sensitive());
}

#[cfg(feature = "remove_all")]
#[test]
fn single_value_removal_round_trips_like_remove() {
    let mut map = HeaderMap::new();
    map.insert(COOKIE, HeaderValue::from_static("only=one"));

    let removed = map.remove_all(COOKIE);
    assert_eq!(removed.len(), 1);
    assert_eq!(removed[0], "only=one");
    assert!(map.is_empty());
    assert!(map.remove_all(COOKIE).is_empty());
}

#[cfg(feature = "remove_all")]
#[test]
fn acceptance_demo_prints_removal_and_round_trip() {
    // Tiny, in-memory fixture (no files, no network).
    let mut map = build_fixture();

    eprintln!("== before remove_all ==");
    for (k, v) in map.iter() {
        eprintln!("  {}: {:?} sensitive={}", k, v, v.is_sensitive());
    }

    let removed = map.remove_all("CoOkIe"); // mixed-case lookup
    eprintln!("== remove_all(\"CoOkIe\") returned, in order ==");
    for (i, v) in removed.iter().enumerate() {
        eprintln!("  [{}] {:?} sensitive={}", i, v, v.is_sensitive());
    }

    eprintln!("== map after removal ==");
    for (k, v) in map.iter() {
        eprintln!("  {}: {:?}", k, v);
    }
    assert_eq!(map.len(), 2);
    assert_eq!(map.keys_len(), 1);

    for value in removed {
        map.append(COOKIE, value);
    }

    eprintln!("== after appending the returned values back ==");
    let cookies = map.get_all(COOKIE);
    for (i, v) in cookies.iter().enumerate() {
        eprintln!("  cookie[{}] {:?} sensitive={}", i, v, v.is_sensitive());
    }
    assert_eq!(cookies.iter().count(), original_cookie_order().len());
}

/// Without the feature enabled, the crate must compile and the legacy
/// interface must behave exactly as before.
#[cfg(not(feature = "remove_all"))]
#[test]
fn legacy_remove_still_works_without_feature() {
    let mut map = build_fixture();

    // `remove` returns the first value and drops the rest, unchanged behavior.
    let first = map.remove(COOKIE).expect("legacy remove returns a value");
    assert_eq!(first, "a=1");
    assert!(map.get(COOKIE).is_none());

    // The new API is simply unavailable; other headers remain intact.
    assert_eq!(map.get_all(SET_COOKIE).iter().count(), 2);
}
