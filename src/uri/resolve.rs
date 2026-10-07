//! Resolution of relative URI references against a base `Uri`.
//!
//! This implements the reference resolution rules of RFC 3986 section 5.2
//! adapted to HTTP URIs: the scheme, authority, path and query are inherited
//! from the base URI as needed, and literal dot segments (`.` and `..`) are
//! removed from the resulting path. Percent-encoded dots (e.g. `%2e`) are
//! *not* decoded and therefore never act as path navigation. Fragments are
//! not part of an HTTP request-target, so a reference containing `#` is
//! rejected explicitly with an error instead of being silently dropped.

use std::convert::TryFrom;

use super::{Authority, ErrorKind, InvalidUri, Parts, PathAndQuery, Uri};

impl Uri {
    /// Resolve a URI reference against this URI, used as the base.
    ///
    /// The reference may be absolute, network-path relative (`//host/...`),
    /// absolute-path relative (`/...`), query-only (`?q`), empty, or a
    /// relative path. Components not present in the reference are inherited
    /// from the base URI, and literal dot segments in the resulting path are
    /// removed. Percent-encoded dot segments such as `%2e` are left untouched
    /// and do not navigate the path hierarchy.
    ///
    /// # Errors
    ///
    /// Returns an error if the reference contains a fragment (`#`), which
    /// HTTP URIs do not support, or if the reference (or the resulting URI)
    /// is otherwise invalid, e.g. a malformed authority.
    ///
    /// # Examples
    ///
    /// ```
    /// use http::Uri;
    ///
    /// let base: Uri = "http://example.com/a/b/c?y=1".parse().unwrap();
    ///
    /// assert_eq!(base.resolve("../x").unwrap(), "http://example.com/a/x");
    /// assert_eq!(base.resolve("?q").unwrap(), "http://example.com/a/b/c?q");
    /// assert_eq!(base.resolve("").unwrap(), "http://example.com/a/b/c?y=1");
    /// // Percent-encoded dots are not path navigation:
    /// assert_eq!(
    ///     base.resolve("%2e/%2E%2E/x").unwrap(),
    ///     "http://example.com/a/b/%2e/%2E%2E/x"
    /// );
    /// // Fragments are rejected, not silently dropped:
    /// assert!(base.resolve("x#frag").is_err());
    /// ```
    pub fn resolve(&self, reference: &str) -> Result<Uri, InvalidUri> {
        resolve(self, reference)
    }
}

fn resolve(base: &Uri, reference: &str) -> Result<Uri, InvalidUri> {
    // HTTP URIs do not carry a fragment component; reject it explicitly
    // rather than silently truncating it.
    if reference.contains('#') {
        return Err(ErrorKind::FragmentNotAllowed.into());
    }

    // An empty reference inherits everything from the base.
    if reference.is_empty() {
        return Ok(base.clone());
    }

    // 1. Absolute reference (has its own scheme): parse it with the existing
    //    `Uri` parser, then remove dot segments from its path.
    if has_scheme(reference) {
        let uri = Uri::try_from(reference)?;
        return remove_dots_from_uri(uri);
    }

    let scheme = base.scheme().cloned();
    let authority = base.authority().cloned();

    // 2. Network-path reference: `//authority/path?query`.
    if let Some(rest) = reference.strip_prefix("//") {
        let end = rest.find(|c| c == '/' || c == '?').unwrap_or(rest.len());
        let (auth, tail) = rest.split_at(end);

        // Reuse the existing authority parser, keeping its precise errors.
        let authority = Authority::try_from(auth)?;

        let path_and_query = if tail.is_empty() {
            PathAndQuery::slash()
        } else {
            path_and_query_from(tail)?
        };

        return from_parts(scheme, Some(authority), path_and_query);
    }

    // 3. Absolute-path reference: `/path?query`.
    if reference.starts_with('/') {
        let path_and_query = path_and_query_from(reference)?;
        return from_parts(scheme, authority, path_and_query);
    }

    // 4. Query-only reference: `?query` inherits the base path.
    if let Some(query) = reference.strip_prefix('?') {
        let mut buf = String::with_capacity(base.path().len() + 1 + query.len());
        buf.push_str(base.path());
        buf.push('?');
        buf.push_str(query);
        let path_and_query = PathAndQuery::try_from(buf)?;
        return from_parts(scheme, authority, path_and_query);
    }

    // 5. Relative-path reference: merge with the base path, then remove dot
    //    segments.
    let (ref_path, ref_query) = split_query(reference);

    let base_path = base.path();
    let mut merged = String::with_capacity(base_path.len() + 1 + ref_path.len());
    if authority.is_some() && base_path.is_empty() {
        // Base has an authority but an empty path: merge against "/".
        merged.push('/');
    } else if let Some(i) = base_path.rfind('/') {
        // Everything up to and including the last '/' of the base path.
        merged.push_str(&base_path[..=i]);
    }
    merged.push_str(ref_path);

    let mut buf = remove_dot_segments(&merged);
    if let Some(query) = ref_query {
        buf.push('?');
        buf.push_str(query);
    }

    let path_and_query = PathAndQuery::try_from(buf)?;
    from_parts(scheme, authority, path_and_query)
}

fn from_parts(
    scheme: Option<super::Scheme>,
    authority: Option<Authority>,
    path_and_query: PathAndQuery,
) -> Result<Uri, InvalidUri> {
    Uri::from_parts(Parts {
        scheme,
        authority,
        path_and_query: Some(path_and_query),
        _priv: (),
    })
    .map_err(|e| e.0)
}

/// Returns true if `s` starts with a scheme component
/// (`scheme = ALPHA *( ALPHA / DIGIT / "+" / "-" / "." ) ":"`), per RFC 3986.
fn has_scheme(s: &str) -> bool {
    let bytes = s.as_bytes();
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

/// Splits a reference into its path and query components.
fn split_query(s: &str) -> (&str, Option<&str>) {
    match s.find('?') {
        Some(i) => (&s[..i], Some(&s[i + 1..])),
        None => (s, None),
    }
}

/// Builds a `PathAndQuery` from a path (starting with `/`) plus optional
/// query, removing literal dot segments from the path portion.
fn path_and_query_from(s: &str) -> Result<PathAndQuery, InvalidUri> {
    let (path, query) = split_query(s);
    let mut buf = remove_dot_segments(path);
    if let Some(query) = query {
        buf.push('?');
        buf.push_str(query);
    }
    PathAndQuery::try_from(buf)
}

/// Removes dot segments from the path of an already-parsed `Uri`, if any.
fn remove_dots_from_uri(uri: Uri) -> Result<Uri, InvalidUri> {
    if !has_dot_segments(uri.path()) {
        return Ok(uri);
    }

    let mut buf = remove_dot_segments(uri.path());
    if let Some(query) = uri.query() {
        buf.push('?');
        buf.push_str(query);
    }

    let mut parts = uri.into_parts();
    parts.path_and_query = Some(PathAndQuery::try_from(buf)?);
    Uri::from_parts(parts).map_err(|e| e.0)
}

/// Cheap check whether a path contains any literal dot segments.
fn has_dot_segments(path: &str) -> bool {
    path.split('/').any(|seg| seg == "." || seg == "..")
}

/// Removes literal dot segments from a path, per RFC 3986 section 5.2.4.
///
/// Only literal `.` and `..` segments are considered; percent-encoded
/// sequences such as `%2e` are compared byte-by-byte and therefore never
/// treated as path navigation.
fn remove_dot_segments(path: &str) -> String {
    let mut input = path;
    let mut output = String::with_capacity(path.len());

    while !input.is_empty() {
        if let Some(rest) = input.strip_prefix("../") {
            input = rest;
        } else if let Some(rest) = input.strip_prefix("./") {
            input = rest;
        } else if input.starts_with("/./") {
            // Replace "/./" with "/".
            input = &input[2..];
        } else if input == "/." {
            input = "/";
        } else if input.starts_with("/../") {
            // Replace "/../" with "/" and drop the last output segment.
            input = &input[3..];
            pop_last_segment(&mut output);
        } else if input == "/.." {
            input = "/";
            pop_last_segment(&mut output);
        } else if input == "." || input == ".." {
            input = "";
        } else {
            // Move the first path segment (including any leading '/') to the
            // output.
            let start = if input.starts_with('/') { 1 } else { 0 };
            let end = input[start..]
                .find('/')
                .map(|i| i + start)
                .unwrap_or(input.len());
            output.push_str(&input[..end]);
            input = &input[end..];
        }
    }

    output
}

/// Removes the last segment (and its preceding '/') from the output buffer.
fn pop_last_segment(output: &mut String) {
    match output.rfind('/') {
        Some(i) => output.truncate(i),
        None => output.clear(),
    }
}
