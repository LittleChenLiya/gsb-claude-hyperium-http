use http::Uri;

fn resolve(base: &str, reference: &str) -> String {
    base.parse::<Uri>()
        .unwrap()
        .resolve(reference)
        .unwrap()
        .to_string()
}

#[test]
fn relative_path_with_dot_dot() {
    // merge "/a/b/" + "../x", then remove the dot segment
    assert_eq!(resolve("http://a/b/c/d", "../x"), "http://a/b/x");
    assert_eq!(resolve("http://a/b/c/d", "../../x"), "http://a/x");
    assert_eq!(resolve("http://a/b/c/d", "./x"), "http://a/b/c/x");
    assert_eq!(resolve("http://a/b/c/d", "../../../x"), "http://a/x");
}

#[test]
fn query_only_reference() {
    // the base path is kept, the query is replaced
    assert_eq!(resolve("http://a/b/c/d?old", "?q"), "http://a/b/c/d?q");
    assert_eq!(resolve("http://a/b/c/d", "?q"), "http://a/b/c/d?q");
}

#[test]
fn empty_reference_inherits_base() {
    // path and query are inherited from the base
    assert_eq!(resolve("http://a/b/c/d?keep", ""), "http://a/b/c/d?keep");
    assert_eq!(resolve("http://a/b/c/d", ""), "http://a/b/c/d");
}

#[test]
fn percent_encoded_dots_are_not_navigation() {
    // `%2e` is data, not a `.` segment: the path is left as-is
    assert_eq!(
        resolve("http://a/b/c/d", "%2e%2e/x"),
        "http://a/b/c/%2e%2e/x"
    );
    assert_eq!(resolve("http://a/b/c/d", "%2E/x"), "http://a/b/c/%2E/x");
    assert_eq!(
        resolve("http://a/b/c/d", "/a/%2e%2e/x"),
        "http://a/a/%2e%2e/x"
    );
}

#[test]
fn absolute_and_absolute_path_references() {
    assert_eq!(resolve("http://a/b/c", "/x/../y"), "http://a/y");
    assert_eq!(resolve("http://a/b/c", "//other/p"), "http://other/p");
    assert_eq!(
        resolve("http://a/b/c", "https://other/q/../r"),
        "https://other/r"
    );
}

#[test]
fn fragment_is_rejected() {
    let base = "http://a/b/c".parse::<Uri>().unwrap();
    let err = base.resolve("../x#frag").unwrap_err();
    assert_eq!(err.to_string(), "fragment not supported in HTTP URI");
    assert!(base.resolve("#frag").is_err());
}

#[test]
fn invalid_authority_is_rejected() {
    let base = "http://a/b".parse::<Uri>().unwrap();
    // space is not a valid authority character
    let err = base.resolve("//exa mple.com/x").unwrap_err();
    assert_eq!(err.to_string(), "invalid uri character");
    // mismatched brackets are an invalid authority
    let err = base.resolve("//[::1/x").unwrap_err();
    assert_eq!(err.to_string(), "invalid authority");
}
