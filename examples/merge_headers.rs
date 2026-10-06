//! Five-header example for `HeaderMap::merge`.
//!
//! Run locally with:
//!
//! ```sh
//! cargo run --example merge_headers
//! ```
//!
//! The two maps each carry five field names:
//!
//! 1. `set-cookie` — two cookies across the maps, kept as two entries
//!    (never comma joined into one `Set-Cookie`).
//! 2. `accept` — a list field, combined with a comma.
//! 3. `content-length` — both sides agree (`42`), so one value survives.
//! 4. `connection` / `keep-alive` — `Connection: keep-alive` names a
//!    hop-by-hop field, so `keep-alive` is filtered while `connection` is
//!    retained.
//! 5. `x-custom` — unknown field; its multiple values are conservatively
//!    preserved as separate entries.

use http::header::{ACCEPT, CONNECTION, CONTENT_LENGTH, SET_COOKIE};
use http::HeaderMap;

fn main() {
    let mut a = HeaderMap::new();
    a.append(SET_COOKIE, "session=abc".parse().unwrap());
    a.insert(ACCEPT, "text/html".parse().unwrap());
    a.insert(CONTENT_LENGTH, "42".parse().unwrap());
    a.insert(CONNECTION, "keep-alive".parse().unwrap());
    a.append("x-custom", "one".parse().unwrap());

    let mut b = HeaderMap::new();
    b.append(SET_COOKIE, "tracking=xyz".parse().unwrap());
    b.insert(ACCEPT, "application/json".parse().unwrap());
    b.insert(CONTENT_LENGTH, "42".parse().unwrap());
    b.insert("keep-alive", "timeout=5".parse().unwrap());
    b.append("x-custom", "two".parse().unwrap());

    let merged = a.merge(&b).expect("the five-header example merges cleanly");

    println!("set-cookie entries: {}", merged.get_all(SET_COOKIE).iter().count());
    for v in merged.get_all(SET_COOKIE).iter() {
        println!("  set-cookie: {}", v.to_str().unwrap());
    }
    println!("accept: {}", merged[ACCEPT].to_str().unwrap());
    println!("content-length: {}", merged[CONTENT_LENGTH].to_str().unwrap());
    println!("connection: {}", merged[CONNECTION].to_str().unwrap());
    println!("keep-alive present: {}", merged.contains_key("keep-alive"));
    println!("x-custom entries: {}", merged.get_all("x-custom").iter().count());
    for v in merged.get_all("x-custom").iter() {
        println!("  x-custom: {}", v.to_str().unwrap());
    }

    // Inputs are unchanged.
    assert_eq!(a.get_all(SET_COOKIE).iter().count(), 1);
    assert_eq!(b.get_all(SET_COOKIE).iter().count(), 1);
}

