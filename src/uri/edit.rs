//! Explicit edit plans for `Uri`.
//!
//! An [`EditPlan`] describes an explicit edit to an existing [`Uri`]: only
//! the parts that are specified (`scheme`, `authority`, or
//! `path_and_query`) are replaced, every other part is carried over
//! untouched. The new combination of parts is validated *before* a new
//! `Uri` is constructed, so a failed edit never produces a half-built URI
//! and never modifies the original one.
//!
//! The three HTTP request-target forms are handled separately:
//!
//! * **origin-form** (`/path?q=1`) — the path is kept exactly as given.
//! * **absolute-form** (`https://example.com/`) — an empty path is
//!   interpreted as `/`.
//! * **authority-form** (`example.com:443`, used by `CONNECT`) — the path
//!   stays empty; no `/` is ever padded onto it.
//!
//! Percent-encoded octets are treated as raw data: a `%2F` in the path is
//! neither decoded nor re-encoded, and the query string keeps its original
//! order. Userinfo is never implicitly moved into an HTTP authority; a
//! replacement authority containing userinfo is rejected explicitly.
//!
//! Run the tests for this module with:
//!
//! ```notrust
//! cargo test --test uri_edit
//! ```

use std::error::Error;
use std::fmt;

use super::{Authority, InvalidUri, PathAndQuery, Scheme, Uri, MAX_LEN};

/// Identifies which part of a URI an [`InvalidUriEdit`] is associated with.
///
/// Every edit error carries the identity of the part that caused it, so
/// callers can tell a malformed `authority` apart from an illegal
/// combination of parts without parsing the error message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UriPart {
    /// The scheme component, e.g. `https`.
    Scheme,
    /// The authority component, e.g. `example.com:8080`.
    Authority,
    /// The path and query component, e.g. `/a%2Fb?b=1&a=2`.
    PathAndQuery,
    /// The combination of the parts is not a legal URI, even though each
    /// individual part parsed successfully.
    Combination,
}

impl fmt::Display for UriPart {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            UriPart::Scheme => "scheme",
            UriPart::Authority => "authority",
            UriPart::PathAndQuery => "path_and_query",
            UriPart::Combination => "combination",
        })
    }
}

/// An error resulting from a failed attempt to edit a [`Uri`] through an
/// [`EditPlan`].
///
/// The error carries the identity of the part that failed via
/// [`InvalidUriEdit::part`]. When an edit fails, no new `Uri` is
/// constructed and the original `Uri` is left unchanged.
#[derive(Debug)]
pub struct InvalidUriEdit {
    part: UriPart,
    kind: EditErrorKind,
}

#[derive(Debug)]
enum EditErrorKind {
    /// The named part failed to parse.
    InvalidPart(InvalidUri),
    /// The replacement authority contains userinfo (`user@host`), which
    /// must not be moved into an HTTP authority.
    UserinfoNotAllowed,
    /// A scheme was present without an authority.
    AuthorityMissing,
    /// An authority and a path were present without a scheme.
    SchemeMissing,
    /// The combined URI exceeds the maximum length.
    TooLong,
}

impl InvalidUriEdit {
    /// Returns the identity of the URI part this error is associated with.
    pub fn part(&self) -> UriPart {
        self.part
    }

    fn invalid_part(part: UriPart, err: InvalidUri) -> Self {
        InvalidUriEdit {
            part,
            kind: EditErrorKind::InvalidPart(err),
        }
    }

    fn combination(kind: EditErrorKind) -> Self {
        InvalidUriEdit {
            part: UriPart::Combination,
            kind,
        }
    }
}

impl fmt::Display for InvalidUriEdit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.kind {
            EditErrorKind::InvalidPart(err) => write!(f, "invalid {}: {}", self.part, err),
            EditErrorKind::UserinfoNotAllowed => {
                write!(f, "userinfo is not allowed in an HTTP authority")
            }
            EditErrorKind::AuthorityMissing => {
                write!(f, "a scheme requires an authority")
            }
            EditErrorKind::SchemeMissing => {
                write!(f, "an authority with a path requires a scheme")
            }
            EditErrorKind::TooLong => write!(f, "uri too long"),
        }
    }
}

impl Error for InvalidUriEdit {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match &self.kind {
            EditErrorKind::InvalidPart(err) => Some(err),
            _ => None,
        }
    }
}

/// An explicit plan to edit selected parts of an existing [`Uri`].
///
/// A plan is created with [`Uri::edit`]. Each setter replaces exactly one
/// part — `scheme`, `authority`, or `path_and_query` — and leaves every
/// unspecified part of the base URI untouched. [`EditPlan::build`]
/// validates the new combination of parts and only then constructs the new
/// `Uri`; the base `Uri` is never modified.
///
/// Parts are parsed from their raw text representation, so percent-encoded
/// octets such as `%2F` are preserved as-is and never double-encoded, and
/// the query string keeps its original order.
///
/// # Examples
///
/// The three HTTP request-target forms, edited explicitly:
///
/// origin-form:
///
/// ```
/// use http::Uri;
///
/// let uri: Uri = "/foo/bar?b=1&a=2".parse().unwrap();
/// let uri = uri.edit().path_and_query("/baz?b=1&a=2").unwrap().build().unwrap();
///
/// assert_eq!(uri.path(), "/baz");
/// // The query keeps its original order.
/// assert_eq!(uri.query(), Some("b=1&a=2"));
/// ```
///
/// absolute-form:
///
/// ```
/// use http::Uri;
///
/// let uri: Uri = "http://example.com/a%2Fb?b=1&a=2".parse().unwrap();
/// let uri = uri.edit()
///     .scheme("https").unwrap()
///     .authority("new.example:8443").unwrap()
///     .build().unwrap();
///
/// // The raw percent-encoded path is not re-encoded.
/// assert_eq!(uri.to_string(), "https://new.example:8443/a%2Fb?b=1&a=2");
/// ```
///
/// authority-form (`CONNECT`):
///
/// ```
/// use http::Uri;
///
/// let uri: Uri = "example.com:443".parse().unwrap();
/// let uri = uri.edit().authority("other.example:8443").unwrap().build().unwrap();
///
/// // No "/" is padded onto an authority-form URI.
/// assert_eq!(uri.to_string(), "other.example:8443");
/// assert_eq!(uri.path(), "");
/// ```
#[derive(Debug)]
pub struct EditPlan<'a> {
    base: &'a Uri,
    scheme: Option<Scheme>,
    authority: Option<Authority>,
    path_and_query: Option<PathAndQuery>,
}

impl<'a> EditPlan<'a> {
    pub(super) fn new(base: &'a Uri) -> Self {
        EditPlan {
            base,
            scheme: None,
            authority: None,
            path_and_query: None,
        }
    }

    /// Replace the scheme part, e.g. `https`.
    ///
    /// Only this part is replaced; the authority and path-and-query of the
    /// base URI are carried over untouched.
    pub fn scheme(mut self, scheme: &str) -> Result<Self, InvalidUriEdit> {
        let scheme = scheme
            .parse::<Scheme>()
            .map_err(|err| InvalidUriEdit::invalid_part(UriPart::Scheme, err))?;
        self.scheme = Some(scheme);
        Ok(self)
    }

    /// Replace the authority part, e.g. `example.com:8080`.
    ///
    /// The authority is replaced as a whole: nothing (in particular no
    /// userinfo) is implicitly moved from the old authority into the new
    /// one. A replacement authority that itself contains userinfo
    /// (`user@host`) is rejected, because userinfo must not appear in an
    /// HTTP authority.
    pub fn authority(mut self, authority: &str) -> Result<Self, InvalidUriEdit> {
        let authority = authority
            .parse::<Authority>()
            .map_err(|err| InvalidUriEdit::invalid_part(UriPart::Authority, err))?;
        if authority.as_str().contains('@') {
            return Err(InvalidUriEdit {
                part: UriPart::Authority,
                kind: EditErrorKind::UserinfoNotAllowed,
            });
        }
        self.authority = Some(authority);
        Ok(self)
    }

    /// Replace the path-and-query part, e.g. `/a%2Fb?b=1&a=2`.
    ///
    /// The raw text is stored as given: percent-encoded octets are not
    /// decoded or re-encoded, and the query keeps its original order. An
    /// empty string means an explicitly empty path, which
    /// [`EditPlan::build`] interprets according to the resulting
    /// request-target form (`/` for absolute-form, empty for
    /// authority-form).
    pub fn path_and_query(mut self, path_and_query: &str) -> Result<Self, InvalidUriEdit> {
        let path_and_query = if path_and_query.is_empty() {
            PathAndQuery::empty()
        } else {
            path_and_query
                .parse::<PathAndQuery>()
                .map_err(|err| InvalidUriEdit::invalid_part(UriPart::PathAndQuery, err))?
        };
        self.path_and_query = Some(path_and_query);
        Ok(self)
    }

    /// Validate the new combination of parts and, only if it is legal,
    /// construct the new `Uri`.
    ///
    /// The base `Uri` is never modified. If the combination is not legal,
    /// an [`InvalidUriEdit`] identifying the offending part is returned
    /// and no new `Uri` is constructed.
    pub fn build(self) -> Result<Uri, InvalidUriEdit> {
        // Unspecified parts are carried over from the base URI untouched.
        let scheme = match self.scheme {
            Some(scheme) => Some(scheme),
            None => self.base.scheme().cloned(),
        };
        let authority = match self.authority {
            Some(authority) => Some(authority),
            None => self.base.authority().cloned(),
        };
        let path_and_query = match self.path_and_query {
            Some(path_and_query) => Some(path_and_query),
            None => self.base.path_and_query().cloned(),
        };

        // Guard the combined length before constructing anything.
        let len = scheme.as_ref().map_or(0, |s| s.as_str().len() + 3)
            + authority.as_ref().map_or(0, |a| a.as_str().len())
            + path_and_query.as_ref().map_or(0, |p| p.as_str().len());
        if len > MAX_LEN {
            return Err(InvalidUriEdit::combination(EditErrorKind::TooLong));
        }

        match (scheme, authority, path_and_query) {
            (Some(_), None, _) => Err(InvalidUriEdit {
                part: UriPart::Authority,
                kind: EditErrorKind::AuthorityMissing,
            }),
            (Some(scheme), Some(authority), path_and_query) => {
                // absolute-form: an empty path is interpreted as "/".
                let path_and_query = match path_and_query {
                    Some(pq) if !pq.data.is_empty() => pq,
                    _ => PathAndQuery::slash(),
                };
                Ok(Uri {
                    scheme,
                    authority,
                    path_and_query,
                })
            }
            (None, Some(_), Some(pq)) if !pq.data.is_empty() => Err(InvalidUriEdit {
                part: UriPart::Scheme,
                kind: EditErrorKind::SchemeMissing,
            }),
            (None, Some(authority), _) => {
                // authority-form (CONNECT): the path stays empty; no "/"
                // is padded onto it.
                Ok(Uri {
                    scheme: Scheme::empty(),
                    authority,
                    path_and_query: PathAndQuery::empty(),
                })
            }
            (None, None, path_and_query) => {
                // origin-form (or asterisk-form): the path is kept exactly
                // as given.
                Ok(Uri {
                    scheme: Scheme::empty(),
                    authority: Authority::empty(),
                    path_and_query: path_and_query.unwrap_or_else(PathAndQuery::empty),
                })
            }
        }
    }
}
