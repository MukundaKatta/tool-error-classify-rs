# tool-error-classify

Map tool errors into a closed `ErrorKind` enum so agent loops can decide what to do: fix args, retry, back off, or give up.

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
