//! Integration tests that exercise the public API as an external consumer
//! would. The `#[cfg(test)]` unit tests in `src/lib.rs` cover the internal
//! classification tables; these tests focus on the surface that downstream
//! crates actually call.

use tool_error_classify::{
    classify, classify_name, classify_status, ClassifiedError, ErrorInfo, ErrorKind,
};

#[test]
fn public_constructors_and_classify() {
    // from_status
    assert_eq!(
        classify(&ErrorInfo::from_status(429)).kind,
        ErrorKind::RateLimited
    );
    // from_type_name
    assert_eq!(
        classify(&ErrorInfo::from_type_name("UnauthorizedException")).kind,
        ErrorKind::Auth
    );
    // convenience wrappers
    assert_eq!(classify_status(404).kind, ErrorKind::NotFound);
    assert_eq!(classify_name("ValidationError").kind, ErrorKind::UserInput);
}

#[test]
fn classification_precedence_status_then_flags_then_name() {
    // Status code beats both flags and type name.
    let info = ErrorInfo::from_status(401)
        .as_timeout()
        .with_type_name("RateLimitError");
    assert_eq!(classify(&info).kind, ErrorKind::Auth);

    // With no status, the timeout flag beats the type name.
    let info = ErrorInfo::new()
        .as_timeout()
        .with_type_name("RateLimitError");
    assert_eq!(classify(&info).kind, ErrorKind::Timeout);

    // With no status and no flags, the type name is used.
    let info = ErrorInfo::new().with_type_name("RateLimitError");
    assert_eq!(classify(&info).kind, ErrorKind::RateLimited);

    // Connection error (no status, no timeout) is retryable.
    let info = ErrorInfo::new().as_connection_error();
    assert_eq!(classify(&info).kind, ErrorKind::Retryable);
}

#[test]
fn unknown_inputs_fall_through_to_unknown() {
    assert_eq!(classify_status(418).kind, ErrorKind::Unknown);
    assert_eq!(
        classify_name("CompletelyMadeUpError").kind,
        ErrorKind::Unknown
    );
    assert_eq!(classify(&ErrorInfo::new()).kind, ErrorKind::Unknown);
}

#[test]
fn retry_after_and_status_carried_through() {
    let r = classify(&ErrorInfo::from_status(429).with_retry_after(12.5));
    assert_eq!(r.status_code, Some(429));
    assert_eq!(r.retry_after_s, Some(12.5));
}

#[test]
fn is_retryable_matches_on_both_kind_and_result() {
    // Kind-level and result-level helpers agree.
    for code in [429u16, 408, 500, 502, 503, 504, 529] {
        let r = classify_status(code);
        assert!(r.is_retryable(), "status {code} should be retryable");
        assert!(r.kind.is_retryable(), "kind for {code} should be retryable");
    }
    for code in [400u16, 401, 403, 404, 410, 501] {
        let r = classify_status(code);
        assert!(!r.is_retryable(), "status {code} should NOT be retryable");
        assert!(
            !r.kind.is_retryable(),
            "kind for {code} should NOT be retryable"
        );
    }
}

#[test]
fn every_kind_has_a_stable_str_and_nonempty_hint() {
    let kinds = [
        ErrorKind::UserInput,
        ErrorKind::Auth,
        ErrorKind::NotFound,
        ErrorKind::RateLimited,
        ErrorKind::Timeout,
        ErrorKind::Retryable,
        ErrorKind::ExternalPermanent,
        ErrorKind::Internal,
        ErrorKind::Unknown,
    ];
    for k in kinds {
        assert!(!k.as_str().is_empty());
        assert_eq!(k.to_string(), k.as_str(), "Display must match as_str");
        assert!(!k.default_hint().is_empty(), "{k} has an empty hint");
    }
}

#[test]
fn classified_error_is_constructible_and_comparable() {
    let a = classify_status(404);
    let b = ClassifiedError {
        kind: ErrorKind::NotFound,
        hint: ErrorKind::NotFound.default_hint().to_owned(),
        status_code: Some(404),
        retry_after_s: None,
    };
    assert_eq!(a, b);
}

#[cfg(feature = "serde")]
mod serde_support {
    use super::*;

    #[test]
    fn error_kind_round_trips_through_json() {
        let json = serde_json::to_string(&ErrorKind::RateLimited).unwrap();
        // Default serde enum representation uses the variant identifier.
        assert_eq!(json, "\"RateLimited\"");
        let back: ErrorKind = serde_json::from_str(&json).unwrap();
        assert_eq!(back, ErrorKind::RateLimited);
    }

    #[test]
    fn classified_error_round_trips_through_json() {
        let original = classify(&ErrorInfo::from_status(429).with_retry_after(30.0));
        let json = serde_json::to_string(&original).unwrap();
        let restored: ClassifiedError = serde_json::from_str(&json).unwrap();
        assert_eq!(original, restored);
    }

    #[test]
    fn error_info_deserializes_with_defaults() {
        // `#[serde(default)]` means a partial object fills the rest in.
        let info: ErrorInfo = serde_json::from_str(r#"{"status_code":503}"#).unwrap();
        assert_eq!(info.status_code, Some(503));
        assert!(!info.is_timeout);
        assert!(!info.is_connection_error);
        assert_eq!(classify(&info).kind, ErrorKind::Retryable);
    }
}
