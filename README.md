# tool-error-classify

Map tool errors into a closed `ErrorKind` enum so agent loops can decide what to do: fix args, retry, back off, or give up.

The crate is dependency-free by default, classifies in three passes (HTTP status code → native network flags → exception type-name keywords), and never panics.

## Install

```toml
[dependencies]
tool-error-classify = "0.1"
```

Enable `serde` if you want to serialize classification results:

```toml
[dependencies]
tool-error-classify = { version = "0.1", features = ["serde"] }
```

## Usage

```rust
use tool_error_classify::{classify, ErrorInfo, ErrorKind};

// HTTP status code
let r = classify(&ErrorInfo::from_status(429));
assert_eq!(r.kind, ErrorKind::RateLimited);
println!("{}", r.hint);

// Exception type name
let r2 = classify(&ErrorInfo::from_type_name("UnauthorizedException"));
assert_eq!(r2.kind, ErrorKind::Auth);

// With retry-after header
let r3 = classify(&ErrorInfo::from_status(429).with_retry_after(30.0));
println!("wait {}s", r3.retry_after_s.unwrap_or(5.0));
```

### Driving a retry loop

```rust
use tool_error_classify::classify_status;

let result = classify_status(503);
if result.is_retryable() {
    let wait = result.retry_after_s.unwrap_or(2.0);
    // sleep `wait` seconds, then retry...
} else {
    // surface `result.hint` to the model or operator and stop.
}
```

## Classification order

1. **HTTP status code** (if `status_code` is set and recognized).
2. **Native network flags** — `is_timeout` then `is_connection_error`.
3. **Exception type-name keywords** — substring match against a fixed rule table.

Anything that matches no rule resolves to `ErrorKind::Unknown`. An earlier pass
always wins over a later one (e.g. a status code overrides a type name).

## API

| Item | Description |
|---|---|
| `classify(&ErrorInfo) -> ClassifiedError` | Full classification entry point. |
| `classify_status(u16) -> ClassifiedError` | Shorthand for a bare HTTP status. |
| `classify_name(&str) -> ClassifiedError` | Shorthand for an exception type name. |
| `ErrorInfo` | Builder-style input (`from_status`, `from_type_name`, `with_retry_after`, `as_timeout`, `as_connection_error`, …). |
| `ClassifiedError` | Result: `kind`, `hint`, `status_code`, `retry_after_s`, plus `is_retryable()`. |
| `ErrorKind` | Closed category enum with `as_str()`, `default_hint()`, `is_retryable()`, and `Display`. |

## Feature flags

| Feature | Default | Effect |
|---|---|---|
| `serde` | off | Derives `Serialize`/`Deserialize` for `ErrorKind`, `ClassifiedError`, and `ErrorInfo`. |

With `serde` enabled, results round-trip through any `serde` format:

```rust
# #[cfg(feature = "serde")] {
use tool_error_classify::classify_status;
let json = serde_json::to_string(&classify_status(429)).unwrap();
# }
```

## ErrorKind variants

| Kind | Meaning |
|---|---|
| `UserInput` | Bad LLM args — surface to the model |
| `Auth` | Auth failure — stop, alert operator |
| `NotFound` | Resource missing — try different one |
| `RateLimited` | Throttled — back off and retry |
| `Timeout` | Timed out — retry or reduce scope |
| `Retryable` | Transient — retry with backoff |
| `ExternalPermanent` | Hard rejection — stop |
| `Internal` | Bug — bubble up |
| `Unknown` | Unclassified — try once more |

## License

MIT OR Apache-2.0
