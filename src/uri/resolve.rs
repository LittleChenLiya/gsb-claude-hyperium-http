//! Resolution of relative URI references against a base URI.
//!
//! This implements the reference resolution algorithm of RFC 3986
//! (Section 5.2) as it applies to HTTP URIs: the scheme, authority, path
//! and query of the base URI are inherited as needed, and literal dot
//! segments (`.` and `..`) in the resulting path are removed.
//!
//! Notes specific to HTTP URIs:
//!
//! * Fragments are not part of an HTTP request target, so a reference
//!   containing `#` is rejected with an explicit error instead of being
//!   silently truncated.
//! * Percent-encoded dots (e.g. `%2e`, `%2E%2E`) are data, not path
//!   navigation: only literal `.` and `..` segments are removed.

use super::{Authority, ErrorKind, InvalidUri, Parts, PathAndQuery, Uri};

impl Uri {
    /// Resolve a relative URI reference against this URI, used as the base.
    ///
    /// This follows the reference resolution rules of RFC 3986 (Section
    /// 5.2): the scheme, authority, path and query of the base are inherited
    /// by the reference as needed, and literal dot segments (`.` and `..`)
    /// in the resulting path are removed. Percent-encoded dots such as
    /// `%2e` are treated as plain data and never as path navigation.
    ///
    /// Since fragments are not part of an HTTP URI, a reference containing
    /// `#` is rejected with an error.
    ///
    /// # Examples
    ///
    /// ```
    /// use http::Uri;
    ///
    /// let base: Uri = "http://example.com/a/b/c?old".parse().unwrap();
    ///
    /// // Relative paths are merged with the base path and dot
    /// // segments are removed.
    /// assert_eq!(base.resolve("../x").unwrap(), "http://example.com/a/x");
    ///
    /// // A query-only reference keeps the base path.
    /// assert_eq!(base.resolve("?q").unwrap(), "http://example.com/a/b/c?q");
    ///
    /// // An empty reference inherits the base path and query.
    /// assert_eq!(base.resolve("").unwrap(), "http://example.com/a/b/c?old");
    ///
    /// // Percent-encoded dots are not path navigation.
    /// assert_eq!(
    ///     base.resolve("%2e%2e/x").unwrap(),
    ///     "http://example.com/a/b/%2e%2e/x",
    /// );
    ///
    /// // Fragments are not supported by HTTP URIs.
    /// assert!(base.resolve("x#frag").is_err());
    /// ```
    pub fn resolve(&self, reference: &str) -> Result<Uri, InvalidUri> {
        resolve(self, reference)
    }
}

fn resolve(base: &Uri, reference: &str) -> Result<Uri, InvalidUri> {
    // Fragments are not part of an HTTP URI; reject them explicitly
    // instead of silently truncating.
    if reference.contains('#') {
        return Err(ErrorKind::FragmentNotSupported.into());
    }

    // An absolute reference inherits nothing from the base, but its path
    // still gets dot segments removed.
    if has_scheme(reference) {
        let uri = Uri::try_from(reference)?;
        return remove_dot_segments_from_uri(uri);
    }

    let base_pq = base.path_and_query();
    let base_path = base_pq.map_or("", |pq| pq.path());

    let scheme = base.scheme().cloned();
    let mut authority = base.authority().cloned();

    // A reference starting with `//` brings its own authority.
    let (has_own_authority, rest) = match reference.strip_prefix("//") {
        Some(rest) => {
            let end = rest.find(|c| c == '/' || c == '?').unwrap_or(rest.len());
            authority = Some(Authority::from_maybe_shared(rest[..end].to_string())?);
            (true, &rest[end..])
        }
        None => (false, reference),
    };

    // Split the remainder into path and query.
    let (ref_path, ref_query) = match rest.find('?') {
        Some(i) => (&rest[..i], Some(&rest[i + 1..])),
        None => (rest, None),
    };

    let (path, query) = if has_own_authority {
        (remove_dot_segments(ref_path), ref_query)
    } else if ref_path.is_empty() {
        // An empty path inherits the base path; the query is inherited
        // only when the reference does not define one.
        let query = match ref_query {
            Some(q) => Some(q),
            None => base_pq.and_then(|pq| pq.query()),
        };
        (base_path.to_string(), query)
    } else if ref_path.starts_with('/') {
        (remove_dot_segments(ref_path), ref_query)
    } else {
        // Merge the reference path with the base path.
        let merged = if authority.is_some() && base_path.is_empty() {
            format!("/{}", ref_path)
        } else {
            match base_path.rfind('/') {
                Some(i) => format!("{}{}", &base_path[..=i], ref_path),
                None => ref_path.to_string(),
            }
        };
        (remove_dot_segments(&merged), ref_query)
    };

    let path_and_query = if path.is_empty() && query.is_none() {
        // Mirror the existing parse semantics: an absolute URI coerces an
        // empty path to `/`, an authority-form URI has no path at all.
        if scheme.is_some() {
            Some(PathAndQuery::slash())
        } else {
            None
        }
    } else {
        let pq = match query {
            Some(q) => format!("{}?{}", path, q),
            None => path,
        };
        Some(PathAndQuery::from_maybe_shared(pq)?)
    };

    let mut parts = Parts::default();
    parts.scheme = scheme;
    parts.authority = authority;
    parts.path_and_query = path_and_query;

    Uri::from_parts(parts).map_err(|e| e.0)
}

/// Does the reference start with a scheme (`ALPHA *(ALPHA / DIGIT / "+" /
/// "-" / ".") ":"`), making it an absolute URI?
fn has_scheme(reference: &str) -> bool {
    let bytes = reference.as_bytes();
    if bytes.is_empty() || !bytes[0].is_ascii_alphabetic() {
        return false;
    }
    for &b in &bytes[1..] {
        match b {
            b':' => return true,
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'+' | b'-' | b'.' => {}
            _ => return false,
        }
    }
    false
}

/// Remove dot segments from the path of an already parsed URI.
fn remove_dot_segments_from_uri(uri: Uri) -> Result<Uri, InvalidUri> {
    let mut parts = uri.into_parts();
    if let Some(pq) = parts.path_and_query {
        let path = remove_dot_segments(pq.path());
        let pq = match pq.query() {
            Some(q) => format!("{}?{}", path, q),
            None => path,
        };
        parts.path_and_query = Some(if pq.is_empty() {
            PathAndQuery::slash()
        } else {
            PathAndQuery::from_maybe_shared(pq)?
        });
    }
    Uri::from_parts(parts).map_err(|e| e.0)
}

/// Remove literal `.` and `..` segments from a path, per RFC 3986
/// (Section 5.2.4). Percent-encoded dots are left untouched.
fn remove_dot_segments(path: &str) -> String {
    let mut input = path;
    let mut output = String::with_capacity(path.len());

    while !input.is_empty() {
        if let Some(rest) = input.strip_prefix("../") {
            input = rest;
        } else if let Some(rest) = input.strip_prefix("./") {
            input = rest;
        } else if input.strip_prefix("/./").is_some() {
            input = &input[2..];
        } else if input == "/." {
            input = "/";
        } else if input.strip_prefix("/../").is_some() {
            input = &input[3..];
            remove_last_segment(&mut output);
        } else if input == "/.." {
            input = "/";
            remove_last_segment(&mut output);
        } else if input == "." || input == ".." {
            input = "";
        } else {
            // Move the first path segment (including a leading '/') from
            // the input to the output.
            let start = if input.starts_with('/') { 1 } else { 0 };
            let end = input[start..].find('/').map_or(input.len(), |i| start + i);
            output.push_str(&input[..end]);
            input = &input[end..];
        }
    }

    output
}

fn remove_last_segment(output: &mut String) {
    match output.rfind('/') {
        Some(i) => output.truncate(i),
        None => output.clear(),
    }
}
