//! Acceptance tests for `Uri::resolve`: relative URI references resolved
//! against a base URI, with small in-memory fixtures.

use http::Uri;

/// Short base plus the required references: `../x`, `?q`, the empty
/// reference, and a `%2e` path. Results are printed so the actual output
/// can be eyeballed with `--nocapture`.
#[test]
fn resolve_short_base_fixtures() {
    let cases: &[(&str, &str, &str)] = &[
        // (base, reference, expected)
        ("http://a/b/c/d?y", "../x", "http://a/b/x"),
        ("http://a/b/c/d?y", "?q", "http://a/b/c/d?q"),
        ("http://a/b/c/d?y", "", "http://a/b/c/d?y"),
        // Percent-encoded dots are NOT path navigation.
        ("http://a/b/c/d", "%2e/%2E%2E/x", "http://a/b/c/%2e/%2E%2E/x"),
        // Literal dot segments are removed.
        ("http://a/b/c/d", "./x", "http://a/b/c/x"),
        ("http://a/b/c/d", "../../g", "http://a/g"),
        ("http://a/b/c/d", "/g/./h/../i", "http://a/g/i"),
        // Network-path and absolute references.
        ("http://a/b/c/d", "//other.io/p/../q", "http://other.io/q"),
        ("http://a/b/c/d", "https://b.io/e/../f", "https://b.io/f"),
        // Relative base works too.
        ("/b/c/d", "../x", "/b/x"),
    ];

    for &(base, reference, expected) in cases {
        let base: Uri = base.parse().unwrap();
        let resolved = base
            .resolve(reference)
            .unwrap_or_else(|e| panic!("resolve({:?}, {:?}) failed: {}", base, reference, e));

        println!("{:>22} + {:<18} => {}", base, reference, resolved);

        assert_eq!(resolved, expected, "base={:?} reference={:?}", base, reference);
        assert_eq!(resolved.to_string(), expected);
    }
}

#[test]
fn resolve_rejects_fragment() {
    let base: Uri = "http://a/b/c".parse().unwrap();

    for reference in ["x#frag", "#frag", "/a#b", "//h.io/p#f", "https://b.io/#f"] {
        let err = base
            .resolve(reference)
            .expect_err("fragment must be rejected");
        println!("{:>22} + {:<12} => error: {}", base, reference, err);
        assert_eq!(err.to_string(), "fragment not allowed");
    }
}

#[test]
fn resolve_bad_authority_keeps_parse_error() {
    let base: Uri = "http://a/b/c".parse().unwrap();

    // Space is not a valid URI character inside an authority.
    let err = base.resolve("//exa mple.com/x").expect_err("bad authority");
    println!("bad authority => error: {}", err);
    assert_eq!(err.to_string(), "invalid uri character");

    // Empty authority in a network-path reference.
    let err = base.resolve("///x").expect_err("empty authority");
    println!("empty authority => error: {}", err);
    assert_eq!(err.to_string(), "empty string");
}

#[test]
fn resolve_empty_and_query_only_inherit_base() {
    let base: Uri = "http://a/b/c/d?y".parse().unwrap();

    let empty = base.resolve("").unwrap();
    assert_eq!(empty, base);
    assert_eq!(empty.query(), Some("y"));

    let query_only = base.resolve("?q").unwrap();
    assert_eq!(query_only.path(), "/b/c/d");
    assert_eq!(query_only.query(), Some("q"));
}
