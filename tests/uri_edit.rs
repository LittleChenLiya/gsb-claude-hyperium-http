//! Acceptance tests for explicit URI edit plans (`Uri::edit`).
//!
//! Run with: cargo test --test uri_edit

use http::uri::UriPart;
use http::Uri;

#[test]
fn percent_encoded_path_raw_preserved_after_host_change() {
    let uri: Uri = "http://old.example/a%2Fb/c%2Fd?b=1&a=2".parse().unwrap();

    let new = uri
        .edit()
        .authority("new.example:8080")
        .unwrap()
        .build()
        .unwrap();

    // The raw %2F octets are not decoded or double-encoded.
    assert_eq!(new.to_string(), "http://new.example:8080/a%2Fb/c%2Fd?b=1&a=2");
    assert_eq!(new.path(), "/a%2Fb/c%2Fd");
    // The query keeps its original order.
    assert_eq!(new.query(), Some("b=1&a=2"));
    // The old URI is not modified.
    assert_eq!(uri.to_string(), "http://old.example/a%2Fb/c%2Fd?b=1&a=2");
}

#[test]
fn connect_authority_form_is_not_padded_with_slash() {
    let uri: Uri = "example.com:443".parse().unwrap();

    let new = uri
        .edit()
        .authority("other.example:8443")
        .unwrap()
        .build()
        .unwrap();

    // authority-form stays authority-form: no "/" is padded on.
    assert_eq!(new.to_string(), "other.example:8443");
    assert_eq!(new.path(), "");
    assert!(new.path_and_query().is_none());
    assert!(new.scheme().is_none());
    // The old URI is not modified.
    assert_eq!(uri.to_string(), "example.com:443");
}

#[test]
fn invalid_authority_yields_error_and_no_new_uri() {
    let uri: Uri = "http://example.com/keep?b=1&a=2".parse().unwrap();

    let err = uri.edit().authority("exa mple.com").unwrap_err();
    assert_eq!(err.part(), UriPart::Authority);

    // No new URI was constructed and the old one is unchanged.
    assert_eq!(uri.to_string(), "http://example.com/keep?b=1&a=2");
}

#[test]
fn invalid_combination_is_rejected_before_construction() {
    // origin-form + authority without a scheme is not a legal URI.
    let uri: Uri = "/path?b=1&a=2".parse().unwrap();

    let err = uri
        .edit()
        .authority("example.com")
        .unwrap()
        .build()
        .unwrap_err();
    assert_eq!(err.part(), UriPart::Scheme);

    // scheme without an authority is not a legal URI either.
    let uri: Uri = "/path".parse().unwrap();
    let err = uri.edit().scheme("https").unwrap().build().unwrap_err();
    assert_eq!(err.part(), UriPart::Authority);
}

#[test]
fn userinfo_is_not_moved_into_http_authority() {
    let uri: Uri = "http://example.com/".parse().unwrap();

    let err = uri.edit().authority("user:pw@example.com").unwrap_err();
    assert_eq!(err.part(), UriPart::Authority);
    assert_eq!(err.to_string(), "userinfo is not allowed in an HTTP authority");

    assert_eq!(uri.to_string(), "http://example.com/");
}

#[test]
fn empty_path_is_interpreted_by_form() {
    // absolute-form: an empty path is interpreted as "/".
    let uri: Uri = "http://example.com".parse().unwrap();
    let new = uri.edit().scheme("https").unwrap().build().unwrap();
    assert_eq!(new.to_string(), "https://example.com/");

    let new = uri
        .edit()
        .path_and_query("")
        .unwrap()
        .build()
        .unwrap();
    assert_eq!(new.to_string(), "http://example.com/");

    // authority-form (CONNECT): an empty path stays empty.
    let uri: Uri = "example.com:443".parse().unwrap();
    let new = uri
        .edit()
        .path_and_query("")
        .unwrap()
        .build()
        .unwrap();
    assert_eq!(new.to_string(), "example.com:443");
    assert_eq!(new.path(), "");
}

#[test]
fn only_specified_parts_are_replaced() {
    let uri: Uri = "https://example.com:8443/p?b=1&a=2".parse().unwrap();

    let new = uri.edit().path_and_query("/other").unwrap().build().unwrap();
    assert_eq!(new.to_string(), "https://example.com:8443/other");

    let new = uri.edit().scheme("http").unwrap().build().unwrap();
    assert_eq!(new.to_string(), "http://example.com:8443/p?b=1&a=2");
}

#[test]
fn three_request_target_forms() {
    // origin-form
    let uri: Uri = "/foo/bar?b=1&a=2".parse().unwrap();
    let uri = uri
        .edit()
        .path_and_query("/baz?b=1&a=2")
        .unwrap()
        .build()
        .unwrap();
    assert_eq!(uri.to_string(), "/baz?b=1&a=2");

    // absolute-form
    let uri: Uri = "http://example.com/a%2Fb?b=1&a=2".parse().unwrap();
    let uri = uri
        .edit()
        .scheme("https")
        .unwrap()
        .authority("new.example:8443")
        .unwrap()
        .build()
        .unwrap();
    assert_eq!(uri.to_string(), "https://new.example:8443/a%2Fb?b=1&a=2");

    // authority-form (CONNECT)
    let uri: Uri = "example.com:443".parse().unwrap();
    let uri = uri
        .edit()
        .authority("other.example:8443")
        .unwrap()
        .build()
        .unwrap();
    assert_eq!(uri.to_string(), "other.example:8443");
}

#[test]
fn invalid_part_errors_carry_part_identity() {
    let uri: Uri = "http://example.com/".parse().unwrap();

    let err = uri.edit().scheme("not a scheme").unwrap_err();
    assert_eq!(err.part(), UriPart::Scheme);

    let err = uri.edit().path_and_query("no-leading-slash").unwrap_err();
    assert_eq!(err.part(), UriPart::PathAndQuery);

    assert_eq!(uri.to_string(), "http://example.com/");
}
