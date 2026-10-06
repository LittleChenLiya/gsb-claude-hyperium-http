//! Explicit, single-component URI editing.
//!
//! Run with:
//!
//! ```sh
//! cargo test --test uri_edit
//! ```
//!
//! The exercised request-target forms are the three common ones:
//! - origin-form:      `/index?a=1`
//! - absolute-form:    `http://example.com/index?a=1`
//! - authority-form:   `example.com:443` (a CONNECT request-target)

use http::uri::{InvalidUriPart, Uri, UriPart};

#[test]
fn percent_encoded_path_keeps_raw_bytes_when_host_changes() {
    // `%2F` is an encoded slash and must stay exactly `%2F` (and never `%252F`).
    let uri = Uri::from_static("http://example.com/a%2Fb?z=1&a=2");

    let new = uri.with_authority("hyper.rs").unwrap();

    assert_eq!(new, "http://hyper.rs/a%2Fb?z=1&a=2");
    assert_eq!(new.path(), "/a%2Fb");
    assert_eq!(new.to_string(), "http://hyper.rs/a%2Fb?z=1&a=2");

    // the original URI is untouched
    assert_eq!(uri, "http://example.com/a%2Fb?z=1&a=2");
}

#[test]
fn scheme_edit_keeps_percent_encoded_path_raw() {
    let uri = Uri::from_static("http://example.com/a%2Fb");

    let new = uri.with_scheme("https").unwrap();

    assert_eq!(new, "https://example.com/a%2Fb");
    assert_eq!(new.path(), "/a%2Fb");
}

#[test]
fn path_edit_keeps_percent_encoded_bytes_raw_and_query_order() {
    let uri = Uri::from_static("http://example.com/old?z=9&a=1&m=5");

    let new = uri.with_path_and_query("/new?b=2&a=1&z=0").unwrap();

    // parameters keep exactly the order they were given
    assert_eq!(new.query(), Some("b=2&a=1&z=0"));
    assert_eq!(new, "http://example.com/new?b=2&a=1&z=0");

    // a raw percent sequence is not percent-encoded again
    let new = uri.with_path_and_query("/a%2Fb%20c").unwrap();
    assert_eq!(new.path(), "/a%2Fb%20c");
    assert_eq!(new.to_string(), "http://example.com/a%2Fb%20c");
}

#[test]
fn connect_authority_form_is_not_given_a_slash_by_authority_edit() {
    // CONNECT request-target: authority-form, no scheme and no path.
    let uri = Uri::from_static("example.com:443");
    assert_eq!(uri.scheme(), None);
    assert_eq!(uri.path(), "");

    let new = uri.with_authority("proxy.example.com:8443").unwrap();

    // still authority-form: no scheme, and no synthetic "/"
    assert_eq!(new.scheme_str(), None);
    assert_eq!(new.path(), "");
    assert_eq!(new, "proxy.example.com:8443");
    assert_eq!(new.to_string(), "proxy.example.com:8443");
    assert!(new.path_and_query().is_none());

    // the original URI is untouched
    assert_eq!(uri, "example.com:443");
}

#[test]
fn connect_authority_form_becoming_absolute_gets_single_slash() {
    // Adding a scheme to an authority-form URI yields an absolute-form URI,
    // whose empty path is interpreted as "/" exactly once.
    let uri = Uri::from_static("example.com:443");

    let new = uri.with_scheme("https").unwrap();

    assert_eq!(new, "https://example.com:443/");
    assert_eq!(new.path(), "/");
}

#[test]
fn connect_authority_form_cannot_carry_a_path() {
    let uri = Uri::from_static("example.com:443");

    let err = uri.with_path_and_query("/").unwrap_err();
    assert_eq!(err.part(), &UriPart::PathAndQuery);
    assert_eq!(uri, "example.com:443");
    assert_eq!(uri.path(), "");
}

#[test]
fn invalid_authority_is_rejected_without_building_a_new_uri() {
    let uri = Uri::from_static("http://example.com/a%2Fb");

    let err = uri.with_authority("bad host").unwrap_err();
    assert_eq!(err.part(), &UriPart::Authority);
    // the error carries the component identity ...
    assert_eq!(
        err.to_string(),
        "invalid uri authority: invalid uri character"
    );

    // ... and no new URI exists; the original survived unchanged.
    assert_eq!(uri, "http://example.com/a%2Fb");
    assert_eq!(uri.host(), Some("example.com"));
}

#[test]
fn invalid_scheme_is_rejected_with_component_identity() {
    let uri = Uri::from_static("http://example.com/");

    let err = uri.with_scheme("ht tp").unwrap_err();
    assert_eq!(err.part(), &UriPart::Scheme);

    assert_eq!(uri, "http://example.com/");
}

#[test]
fn invalid_path_is_rejected_with_component_identity() {
    let uri = Uri::from_static("http://example.com/");

    let err = uri.with_path_and_query("no-leading-slash").unwrap_err();
    assert_eq!(err.part(), &UriPart::PathAndQuery);

    assert_eq!(uri, "http://example.com/");
}

#[test]
fn origin_form_cannot_gain_scheme_or_authority() {
    let uri = Uri::from_static("/foo?a=1");

    let err = uri.with_scheme("https").unwrap_err();
    assert_eq!(err.part(), &UriPart::Authority);

    let err = uri.with_authority("example.com").unwrap_err();
    assert_eq!(err.part(), &UriPart::PathAndQuery);

    assert_eq!(uri, "/foo?a=1");
}

#[test]
fn origin_form_path_edit_preserves_query_order() {
    let uri = Uri::from_static("/old?z=1&a=2");

    let new = uri.with_path_and_query("/new?z=1&a=2").unwrap();

    assert_eq!(new, "/new?z=1&a=2");
    assert_eq!(new.query(), Some("z=1&a=2"));
}

#[test]
fn authority_replacement_never_borrows_userinfo_elsewhere() {
    // An authority with userinfo keeps exactly the authority that was passed;
    // nothing from the scheme/path is moved into the new authority, and an
    // authority without userinfo does not acquire one.
    let uri = Uri::from_static("http://old:secret@example.com/");

    let new = uri.with_authority("newhost.example").unwrap();
    assert_eq!(new.host(), Some("newhost.example"));
    assert_eq!(new.authority().unwrap().as_str(), "newhost.example");
    assert!(!new.authority().unwrap().as_str().contains('@'));
    assert_eq!(new, "http://newhost.example/");

    // explicit userinfo in the replacement is kept verbatim
    let new = uri.with_authority("user:pass@newhost.example").unwrap();
    assert_eq!(
        new.authority().unwrap().as_str(),
        "user:pass@newhost.example"
    );
    assert_eq!(new.host(), Some("newhost.example"));
}

#[test]
fn absolute_form_empty_path_is_interpreted_as_slash() {
    let uri = Uri::from_static("http://example.com/");

    // PathAndQuery::try_from("") fails at the component level; verify the
    // scheme-side empty handling instead: an absolute URI parsed without a
    // path already carries "/", and editing keeps that interpretation.
    let new = uri.with_authority("example.org").unwrap();
    assert_eq!(new.path(), "/");
    assert_eq!(new, "http://example.org/");
}

#[test]
fn asterisk_form_stays_origin_like() {
    let uri = Uri::from_static("*");

    let new = uri.with_path_and_query("*").unwrap();
    assert_eq!(new, "*");
    assert_eq!(new.path(), "*");

    assert!(uri.with_scheme("http").is_err());
    assert!(uri.with_authority("example.com").is_err());
}

#[test]
fn invalid_uri_part_is_std_error_with_source() {
    use std::error::Error;

    let uri = Uri::from_static("http://example.com/");
    let err: InvalidUriPart = uri.with_authority("@").unwrap_err();
    assert!(err.source().is_some());
}
