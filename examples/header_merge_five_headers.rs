//! The five-header merge example.
//!
//! Run locally with:
//!
//! ```sh
//! cargo run --example header_merge_five_headers
//! ```
//!
//! Five headers show every branch of the explicit merge plan:
//! 1. `accept`          - a list header, merged with commas;
//! 2. `content-length`  - a single-value header that must agree;
//! 3. `set-cookie`      - a non-mergeable multi-value header kept as entries;
//! 4. `connection` + `keep-alive` - connection-named fields filtered out;
//! 5. `x-custom`        - an unknown field whose values are kept as-is.

use http::header::{merge, ACCEPT, CONNECTION, CONTENT_LENGTH, HOST, SET_COOKIE};
use http::{HeaderMap, HeaderName};

fn hv(value: &str) -> http::HeaderValue {
    value.parse().unwrap()
}

fn main() {
    // --- Response/header block A -------------------------------------------
    let mut a = HeaderMap::new();
    // 1. list
    a.insert(ACCEPT, hv("text/html"));
    // 2. single value
    a.insert(CONTENT_LENGTH, hv("1234"));
    // 3. non-mergeable multi-value
    a.append(SET_COOKIE, hv("session=aaa; Path=/"));
    // 4. connection-specific: "keep-alive" names the next header
    a.append(CONNECTION, hv("keep-alive"));
    a.insert(HeaderName::from_static("keep-alive"), hv("timeout=5"));
    // 5. unknown custom field
    a.append(HeaderName::from_static("x-custom"), hv("p=1"));
    // an unrelated singleton
    a.insert(HOST, hv("example.com"));

    // --- Response/header block B -------------------------------------------
    let mut b = HeaderMap::new();
    b.insert(ACCEPT, hv("application/json"));
    b.insert(HeaderName::from_static("content-length"), hv("1234"));
    b.append(SET_COOKIE, hv("tracking=bbb; Path=/"));
    b.append(HeaderName::from_static("x-custom"), hv("q=2"));

    let merged = merge(&a, &b).expect("the two blocks are consistent");

    println!("-- merged headers --");
    for (name, value) in &merged {
        println!("{name}: {}", value.to_str().unwrap());
    }

    // Assertions describing the expected result.
    assert_eq!(merged[ACCEPT], "text/html, application/json");
    assert_eq!(merged[CONTENT_LENGTH], "1234");
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
    assert_eq!(merged[HOST], "example.com");

    println!("ok: five-header merge behaves per the explicit plan");
}
