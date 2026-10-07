//! Small in-memory demonstration of `HeaderMap::remove_all`.
//!
//! Run with:
//!     cargo run --features remove_all --example remove_all_demo
//!
//! The fixture builds a `HeaderMap` containing duplicate `Set-Cookie` values
//! inserted under different casings (header names are matched by identity,
//! i.e. case-insensitively), one of which is marked sensitive. It then:
//!   1. removes every cookie and prints the removal order + sensitive flags;
//!   2. prints the headers left in the map;
//!   3. appends the removed values back and prints the restored map, showing
//!      the original order is recovered exactly.

use http::header::{HeaderValue, VARY, VIA};

fn dump(map: &http::HeaderMap, label: &str) {
    println!("--- {label} (len={}) ---", map.len());
    for (k, v) in map.iter() {
        let body = if v.is_sensitive() {
            format!("{} [sensitive]", v.to_str().unwrap_or("<non-ascii>"))
        } else {
            v.to_str().unwrap_or("<non-ascii>").to_string()
        };
        println!("  {k}: {body}");
    }
}

fn main() {
    let mut map = http::HeaderMap::new();

    map.append("Set-Cookie", HeaderValue::from_static("cookie_1=value 1"));
    map.append("SET-COOKIE", HeaderValue::from_static("cookie_2=value 2"));
    let mut secret = HeaderValue::from_static("cookie_3=secret");
    secret.set_sensitive(true);
    map.append("set-cookie", secret);

    map.insert(VIA, HeaderValue::from_static("1.1 example.com"));
    map.append("vIa", HeaderValue::from_static("1.1 other.com"));
    map.insert(VARY, HeaderValue::from_static("*"));

    dump(&map, "original map");

    // Different casing, same HeaderName identity: all three cookies removed.
    let removed = map.remove_all("sEt-CoOkIe");

    println!(
        "--- remove_all(\"sEt-CoOkIe\") returned {} value(s) ---",
        removed.len()
    );
    for (i, v) in removed.iter().enumerate() {
        println!(
            "  [{i}] {} sensitive={}",
            v.to_str().unwrap(),
            v.is_sensitive()
        );
    }

    dump(&map, "map after removal");

    // Missing key: empty Vec, map untouched.
    let nothing = map.remove_all("x-absent");
    println!(
        "remove_all(\"x-absent\") -> {} value(s), len stays {}",
        nothing.len(),
        map.len()
    );

    // Put the values back in the returned order.
    for v in removed {
        map.append("set-cookie", v);
    }
    dump(&map, "map after appending removed values back");
}
