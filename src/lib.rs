/*!
tool-error-classify: map tool errors to a closed `ErrorKind` enum.

When a tool errors, the agent loop must decide: fix the args, retry,
back off, or give up. This crate classifies errors based on HTTP status
code, exception-class name keywords, and network-error type flags.

```rust
use tool_error_classify::{classify, ErrorInfo, ErrorKind};

// Got a 429 from an API
let info = ErrorInfo::from_status(429);
let result = classify(&info);
assert_eq!(result.kind, ErrorKind::RateLimited);
println!("{}", result.hint);

// Got a name-based error
let info2 = ErrorInfo::from_type_name("RateLimitError");
let result2 = classify(&info2);
assert_eq!(result2.kind, ErrorKind::RateLimited);
```
*/

// ---- ErrorKind -----------------------------------------------------------

/// Closed enum of tool-error categories.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum ErrorKind {
    /// Bad args from the LLM. Surface message so it can retry.
    UserInput,
    /// Authentication or authorization failure. Stop, alert operator.
    Auth,
    /// Resource doesn't exist. Tell LLM to try a different one.
    NotFound,
    /// Throttled. Back off and retry.
    RateLimited,
    /// Request timed out. Retry with smaller scope or wait.
    Timeout,
    /// Transient error. Retry with backoff.
    Retryable,
    /// External system permanently rejects this request. Stop.
    ExternalPermanent,
    /// Our own bug. Bubble up; retrying won't help.
    Internal,
    /// Couldn't classify. Try once more then give up.
    Unknown,
}

impl ErrorKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::UserInput => "user_input",
            Self::Auth => "auth",
            Self::NotFound => "not_found",
            Self::RateLimited => "rate_limited",
            Self::Timeout => "timeout",
            Self::Retryable => "retryable",
            Self::ExternalPermanent => "external_permanent",
            Self::Internal => "internal",
            Self::Unknown => "unknown",
        }
    }

    /// Whether retrying an error of this kind has any chance of succeeding.
    ///
    /// Returns `true` for [`RateLimited`](Self::RateLimited),
    /// [`Timeout`](Self::Timeout), and [`Retryable`](Self::Retryable); `false`
    /// for everything else.
    ///
    /// ```
    /// use tool_error_classify::ErrorKind;
    /// assert!(ErrorKind::RateLimited.is_retryable());
    /// assert!(!ErrorKind::Auth.is_retryable());
    /// ```
    pub fn is_retryable(self) -> bool {
        matches!(self, Self::RateLimited | Self::Timeout | Self::Retryable)
    }

    pub fn default_hint(self) -> &'static str {
        match self {
            Self::UserInput => "Your tool arguments are invalid. Check the schema and try again.",
            Self::Auth => "Authentication failed. Stop retrying and ask for credentials.",
            Self::NotFound => "The resource you asked for does not exist. Try a different one.",
            Self::RateLimited => "You are being throttled. Wait before retrying.",
            Self::Timeout => "The request timed out. Try a smaller scope or wait.",
            Self::Retryable => "Transient error. Retry with backoff.",
            Self::ExternalPermanent => {
                "The external system rejected this request and will keep rejecting it."
            }
            Self::Internal => {
                "An internal error occurred. This is unlikely to be fixed by retrying."
            }
            Self::Unknown => "An unclassified error occurred. Try once more, then give up.",
        }
    }
}

impl std::fmt::Display for ErrorKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

// ---- ClassifiedError -----------------------------------------------------

/// Result of `classify`.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ClassifiedError {
    pub kind: ErrorKind,
    pub hint: String,
    pub status_code: Option<u16>,
    pub retry_after_s: Option<f64>,
}

impl ClassifiedError {
    /// Whether the agent loop should retry this error. Delegates to
    /// [`ErrorKind::is_retryable`].
    pub fn is_retryable(&self) -> bool {
        self.kind.is_retryable()
    }
}

// ---- ErrorInfo -----------------------------------------------------------

/// Input descriptor for `classify`. Build via `from_*` constructors or set
/// fields directly.
#[derive(Debug, Clone, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(default))]
pub struct ErrorInfo {
    pub status_code: Option<u16>,
    pub type_name: Option<String>,
    pub is_timeout: bool,
    pub is_connection_error: bool,
    pub retry_after_s: Option<f64>,
}

impl ErrorInfo {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_status(code: u16) -> Self {
        Self {
            status_code: Some(code),
            ..Default::default()
        }
    }

    pub fn from_type_name(name: impl Into<String>) -> Self {
        Self {
            type_name: Some(name.into()),
            ..Default::default()
        }
    }

    pub fn with_status(mut self, code: u16) -> Self {
        self.status_code = Some(code);
        self
    }

    pub fn with_type_name(mut self, name: impl Into<String>) -> Self {
        self.type_name = Some(name.into());
        self
    }

    pub fn with_retry_after(mut self, secs: f64) -> Self {
        self.retry_after_s = Some(secs);
        self
    }

    pub fn as_timeout(mut self) -> Self {
        self.is_timeout = true;
        self
    }

    pub fn as_connection_error(mut self) -> Self {
        self.is_connection_error = true;
        self
    }
}

// ---- classification tables -----------------------------------------------

fn classify_by_status(code: u16) -> Option<ErrorKind> {
    match code {
        400 | 405 | 413 | 414 | 422 => Some(ErrorKind::UserInput),
        401 | 403 => Some(ErrorKind::Auth),
        404 => Some(ErrorKind::NotFound),
        408 => Some(ErrorKind::Timeout),
        409 | 425 | 500 | 502 | 503 => Some(ErrorKind::Retryable),
        410 | 501 => Some(ErrorKind::ExternalPermanent),
        429 | 529 => Some(ErrorKind::RateLimited),
        504 => Some(ErrorKind::Timeout),
        _ => None,
    }
}

fn classify_by_name(name: &str) -> Option<ErrorKind> {
    let rules: &[(&str, ErrorKind)] = &[
        ("RateLimit", ErrorKind::RateLimited),
        ("ThrottlingException", ErrorKind::RateLimited),
        ("ServiceQuotaExceeded", ErrorKind::RateLimited),
        ("Timeout", ErrorKind::Timeout),
        ("Unauthorized", ErrorKind::Auth),
        ("Authentication", ErrorKind::Auth),
        ("PermissionDenied", ErrorKind::Auth),
        ("Forbidden", ErrorKind::Auth),
        ("NotFound", ErrorKind::NotFound),
        ("DoesNotExist", ErrorKind::NotFound),
        ("Overloaded", ErrorKind::Retryable),
        ("ServiceUnavailable", ErrorKind::Retryable),
        ("APIConnectionError", ErrorKind::Retryable),
        ("InternalServer", ErrorKind::Retryable),
        ("ModelStreamErrorException", ErrorKind::Retryable),
        ("BadRequest", ErrorKind::UserInput),
        ("Validation", ErrorKind::UserInput),
        ("InvalidArgument", ErrorKind::UserInput),
    ];
    for &(kw, kind) in rules {
        if name.contains(kw) {
            return Some(kind);
        }
    }
    None
}

// ---- public API ----------------------------------------------------------

/// Classify an error into an `ErrorKind` and a one-line hint.
///
/// Classification order:
/// 1. HTTP status code (if set)
/// 2. Network error flags (`is_timeout`, `is_connection_error`)
/// 3. Type-name keyword matching
pub fn classify(info: &ErrorInfo) -> ClassifiedError {
    let kind = classify_inner(info);
    ClassifiedError {
        kind,
        hint: kind.default_hint().to_owned(),
        status_code: info.status_code,
        retry_after_s: info.retry_after_s,
    }
}

fn classify_inner(info: &ErrorInfo) -> ErrorKind {
    // 1. Status code wins.
    if let Some(code) = info.status_code {
        if let Some(k) = classify_by_status(code) {
            return k;
        }
    }
    // 2. Native network flags.
    if info.is_timeout {
        return ErrorKind::Timeout;
    }
    if info.is_connection_error {
        return ErrorKind::Retryable;
    }
    // 3. Type-name keywords.
    if let Some(name) = &info.type_name {
        if let Some(k) = classify_by_name(name) {
            return k;
        }
    }
    ErrorKind::Unknown
}

/// Convenience: classify a bare HTTP status code.
pub fn classify_status(code: u16) -> ClassifiedError {
    classify(&ErrorInfo::from_status(code))
}

/// Convenience: classify by error type name.
pub fn classify_name(name: &str) -> ClassifiedError {
    classify(&ErrorInfo::from_type_name(name))
}

// ---- tests ---------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_400_user_input() {
        assert_eq!(classify_status(400).kind, ErrorKind::UserInput);
    }

    #[test]
    fn status_401_auth() {
        assert_eq!(classify_status(401).kind, ErrorKind::Auth);
    }

    #[test]
    fn status_403_auth() {
        assert_eq!(classify_status(403).kind, ErrorKind::Auth);
    }

    #[test]
    fn status_404_not_found() {
        assert_eq!(classify_status(404).kind, ErrorKind::NotFound);
    }

    #[test]
    fn status_408_timeout() {
        assert_eq!(classify_status(408).kind, ErrorKind::Timeout);
    }

    #[test]
    fn status_429_rate_limited() {
        assert_eq!(classify_status(429).kind, ErrorKind::RateLimited);
    }

    #[test]
    fn status_500_retryable() {
        assert_eq!(classify_status(500).kind, ErrorKind::Retryable);
    }

    #[test]
    fn status_501_external_permanent() {
        assert_eq!(classify_status(501).kind, ErrorKind::ExternalPermanent);
    }

    #[test]
    fn status_503_retryable() {
        assert_eq!(classify_status(503).kind, ErrorKind::Retryable);
    }

    #[test]
    fn status_504_timeout() {
        assert_eq!(classify_status(504).kind, ErrorKind::Timeout);
    }

    #[test]
    fn status_529_rate_limited() {
        assert_eq!(classify_status(529).kind, ErrorKind::RateLimited);
    }

    #[test]
    fn status_422_user_input() {
        assert_eq!(classify_status(422).kind, ErrorKind::UserInput);
    }

    #[test]
    fn name_rate_limit() {
        assert_eq!(classify_name("RateLimitError").kind, ErrorKind::RateLimited);
    }

    #[test]
    fn name_throttling() {
        assert_eq!(
            classify_name("ThrottlingException").kind,
            ErrorKind::RateLimited
        );
    }

    #[test]
    fn name_timeout() {
        assert_eq!(
            classify_name("RequestTimeoutError").kind,
            ErrorKind::Timeout
        );
    }

    #[test]
    fn name_auth() {
        assert_eq!(classify_name("UnauthorizedError").kind, ErrorKind::Auth);
    }

    #[test]
    fn name_forbidden() {
        assert_eq!(classify_name("ForbiddenException").kind, ErrorKind::Auth);
    }

    #[test]
    fn name_not_found() {
        assert_eq!(classify_name("ResourceNotFound").kind, ErrorKind::NotFound);
    }

    #[test]
    fn name_overloaded() {
        assert_eq!(
            classify_name("ModelOverloadedException").kind,
            ErrorKind::Retryable
        );
    }

    #[test]
    fn name_service_unavailable() {
        assert_eq!(
            classify_name("ServiceUnavailableError").kind,
            ErrorKind::Retryable
        );
    }

    #[test]
    fn name_validation() {
        assert_eq!(classify_name("ValidationError").kind, ErrorKind::UserInput);
    }

    #[test]
    fn name_invalid_argument() {
        assert_eq!(
            classify_name("InvalidArgumentException").kind,
            ErrorKind::UserInput
        );
    }

    #[test]
    fn is_timeout_flag() {
        let info = ErrorInfo::new().as_timeout();
        assert_eq!(classify(&info).kind, ErrorKind::Timeout);
    }

    #[test]
    fn is_connection_error_flag() {
        let info = ErrorInfo::new().as_connection_error();
        assert_eq!(classify(&info).kind, ErrorKind::Retryable);
    }

    #[test]
    fn status_beats_type_name() {
        let info = ErrorInfo::from_status(429).with_type_name("ValidationError");
        assert_eq!(classify(&info).kind, ErrorKind::RateLimited); // status wins
    }

    #[test]
    fn retry_after_carried_through() {
        let info = ErrorInfo::from_status(429).with_retry_after(30.0);
        let r = classify(&info);
        assert_eq!(r.retry_after_s, Some(30.0));
    }

    #[test]
    fn unknown_status_falls_through() {
        let r = classify_status(418); // I'm a teapot
        assert_eq!(r.kind, ErrorKind::Unknown);
    }

    #[test]
    fn unknown_name_falls_through() {
        let r = classify_name("SomeBizarreException");
        assert_eq!(r.kind, ErrorKind::Unknown);
    }

    #[test]
    fn hint_non_empty() {
        for code in [400u16, 401, 403, 404, 408, 429, 500, 501, 503, 504] {
            let r = classify_status(code);
            assert!(!r.hint.is_empty(), "empty hint for status {code}");
        }
    }

    #[test]
    fn is_retryable_check() {
        assert!(classify_status(429).is_retryable());
        assert!(classify_status(503).is_retryable());
        assert!(!classify_status(404).is_retryable());
        assert!(!classify_status(401).is_retryable());
    }

    #[test]
    fn error_kind_display() {
        assert_eq!(ErrorKind::RateLimited.to_string(), "rate_limited");
        assert_eq!(ErrorKind::UserInput.to_string(), "user_input");
    }

    #[test]
    fn builder_chain() {
        let info = ErrorInfo::new()
            .with_status(429)
            .with_type_name("RateLimitError")
            .with_retry_after(5.0);
        let r = classify(&info);
        assert_eq!(r.kind, ErrorKind::RateLimited);
        assert_eq!(r.retry_after_s, Some(5.0));
        assert_eq!(r.status_code, Some(429));
    }
}
