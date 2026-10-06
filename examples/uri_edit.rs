//! Explicit, single-component URI editing across the three common
//! request-target forms.
//!
//! Run with:
//!
//! ```sh
//! cargo run --example uri_edit
//! ```
//!
//! The editing methods only replace the component you name, validate the new
//! combination before building anything, never touch the borrowed original
//! `Uri`, and keep raw percent-encoded bytes (such as `%2F`) intact.

use http::Uri;

fn main() {
    // 1. origin-form: `/path?query` (no scheme, no authority)
    let origin = Uri::from_static("/search?q=1&page=2");
    let retargeted = origin
        .with_path_and_query("/search?q=9&page=3")
        .expect("valid path and query");
    println!("origin-form:  {} -> {}", origin, retargeted);
    assert_eq!(retargeted.to_string(), "/search?q=9&page=3");
    assert_eq!(origin.to_string(), "/search?q=1&page=2");

    // 2. absolute-form: `scheme://authority/path?query`.
    //    Changing the host leaves the raw `%2F` path and the query order alone.
    let absolute = Uri::from_static("http://example.com/a%2Fb?z=1&a=2");
    let rewired = absolute
        .with_scheme("https")
        .and_then(|uri| uri.with_authority("hyper.rs:8443"))
        .expect("valid scheme and authority");
    println!("absolute-form: {} -> {}", absolute, rewired);
    assert_eq!(rewired.to_string(), "https://hyper.rs:8443/a%2Fb?z=1&a=2");

    // 3. authority-form: `host:port`, used by CONNECT.
    //    Replacing the authority does not invent a "/" path.
    let connect = Uri::from_static("proxy.example.com:443");
    let repointed = connect
        .with_authority("upstream.example.com:8443")
        .expect("valid authority");
    println!("authority-form: {} -> {}", connect, repointed);
    assert_eq!(repointed.to_string(), "upstream.example.com:8443");
    assert_eq!(repointed.path(), "");

    // Promoting an authority-form target to absolute-form gives it the
    // mandatory empty-path "/" exactly once.
    let promoted = connect.with_scheme("https").unwrap();
    println!("CONNECT promoted: {} -> {}", connect, promoted);
    assert_eq!(promoted.to_string(), "https://proxy.example.com:443/");

    // An invalid replacement is rejected with the offending component named;
    // the original URI is unchanged and no new URI is produced.
    let err = absolute.with_authority("bad host").unwrap_err();
    println!("rejected edit: {:?} ({})", err.part(), err);
    assert_eq!(absolute.to_string(), "http://example.com/a%2Fb?z=1&a=2");
}
