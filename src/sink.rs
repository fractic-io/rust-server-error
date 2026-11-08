//! A process-wide observer of every `ServerError` at the moment it is
//! constructed, and the record it turns into. Binaries install one sink at
//! startup (a CLI appends to a journal, a Lambda prints a JSON line);
//! libraries never install one. Without a sink, construction costs one
//! atomic load.
//!
//! Capturing at construction rather than where an error is displayed is
//! deliberate: it also sees the errors a caller swallows or downgrades, which
//! are the ones no display hook can report.

use std::{
    panic::{Location, PanicHookInfo},
    sync::OnceLock,
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};

use crate::{ServerErrorBehaviour, ServerErrorTag, ServerErrorTrait};

// Definitions.
// ----------------------------------------------------------------------------

/// What the sink sees. Borrowed from the error being built; copy out what
/// must outlive the call (`ErrorRecord::from_event` does).
#[derive(Debug, Clone, Copy)]
pub struct ErrorEvent<'a> {
    /// `module_path!()::TypeName` of the error struct: the grouping key
    /// across runs, versions and machines.
    pub fingerprint: &'a str,
    /// The unformatted message template, for grouping and triage.
    pub template: &'a str,
    pub kind: ErrorKind,
    /// Where the constructor was called.
    pub location: &'a Location<'a>,
    pub error: &'a dyn ServerErrorTrait,
}

/// The classification carried by the defining macro.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    Critical,
    Internal,
    Client,
    Sensitive,
    User,
}

impl ErrorKind {
    pub fn of(error: &dyn ServerErrorTrait) -> Self {
        match (error.tag(), error.behaviour()) {
            (ServerErrorTag::Critical, _) => Self::Critical,
            (_, ServerErrorBehaviour::ReturnInternalServerError) => Self::Internal,
            (_, ServerErrorBehaviour::ReturnUnauthorized) => Self::Sensitive,
            (_, ServerErrorBehaviour::LogErrorSendFixedMsgToClient(_))
            | (_, ServerErrorBehaviour::LogErrorForwardToClient) => Self::Client,
            _ => Self::User,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Critical => "critical",
            Self::Internal => "internal",
            Self::Client => "client",
            Self::Sensitive => "sensitive",
            Self::User => "user",
        }
    }
}

pub type ErrorSink = Box<dyn Fn(ErrorEvent<'_>) + Send + Sync + 'static>;

/// Where records come from: the same for every record of a process.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ErrorOrigin {
    /// The product and component (`storyengine-cli`, `app-backend`).
    pub source: String,
    /// `sandbox`, `staging` or `production`.
    pub env: Option<String>,
    /// How the process was started (`interactive`, `harness`, a Lambda's
    /// function name).
    pub session: Option<String>,
    pub version: Option<String>,
}

/// One tracked error, as every source writes it: a JSON object per line in a
/// CloudWatch log group, with the same keys whether it came from a Rust
/// process, a Lambda or the app. `v` is the envelope version.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ErrorRecord {
    pub v: u8,
    /// RFC 3339, UTC, second precision.
    pub ts: String,
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub env: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    pub kind: String,
    pub fingerprint: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub template: Option<String>,
    pub message: String,
    /// The `with_debug` cause. Dropped for sensitive errors.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub debug: Option<String>,
    /// `file:line` of the constructor call.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
    /// The captured backtrace, when the error's context is one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backtrace: Option<String>,
}

pub const ERROR_RECORD_VERSION: u8 = 1;
/// The fingerprint of a panic record; its `message` is the panic payload.
pub const PANIC_FINGERPRINT: &str = "panic";

const MAX_MESSAGE_CHARS: usize = 2000;
const MAX_DEBUG_CHARS: usize = 4000;
const MAX_BACKTRACE_CHARS: usize = 4000;

// Public interface.
// ----------------------------------------------------------------------------

static SINK: OnceLock<ErrorSink> = OnceLock::new();

/// Installs the process-wide sink. A second call is ignored and returns
/// `false`; the first sink stays.
pub fn install_error_sink(sink: ErrorSink) -> bool {
    SINK.set(sink).is_ok()
}

/// Called by `define_server_error!` constructors. Not for direct use.
#[doc(hidden)]
pub fn __report(
    fingerprint: &str,
    template: &str,
    location: &Location<'_>,
    error: &dyn ServerErrorTrait,
) {
    if let Some(sink) = SINK.get() {
        sink(ErrorEvent {
            fingerprint,
            template,
            kind: ErrorKind::of(error),
            location,
            error,
        });
    }
}

impl ErrorRecord {
    pub fn from_event(event: &ErrorEvent<'_>, origin: &ErrorOrigin) -> Self {
        let location = format!("{}:{}", event.location.file(), event.location.line());
        let context = event.error.context();
        let plain_context = format!("{}; {};", event.location.file(), event.location.line());
        let backtrace = (context != "OMITTED" && *context != plain_context)
            .then(|| truncate(context, MAX_BACKTRACE_CHARS));
        let debug = match event.kind {
            ErrorKind::Sensitive => None,
            _ => event.error.debug().map(|d| truncate(d, MAX_DEBUG_CHARS)),
        };
        Self {
            v: ERROR_RECORD_VERSION,
            ts: now_rfc3339(),
            source: origin.source.clone(),
            env: origin.env.clone(),
            session: origin.session.clone(),
            version: origin.version.clone(),
            kind: event.kind.as_str().to_owned(),
            fingerprint: event.fingerprint.to_owned(),
            template: Some(event.template.to_owned()),
            message: truncate(event.error.message(), MAX_MESSAGE_CHARS),
            debug,
            location: Some(location),
            backtrace,
        }
    }

    /// A record for a panic, for a `std::panic::set_hook` hook.
    pub fn from_panic(info: &PanicHookInfo<'_>, origin: &ErrorOrigin) -> Self {
        let payload = info.payload();
        let message = payload
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| payload.downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "panic with a non-string payload".to_owned());
        Self {
            v: ERROR_RECORD_VERSION,
            ts: now_rfc3339(),
            source: origin.source.clone(),
            env: origin.env.clone(),
            session: origin.session.clone(),
            version: origin.version.clone(),
            kind: ErrorKind::Critical.as_str().to_owned(),
            fingerprint: PANIC_FINGERPRINT.to_owned(),
            template: None,
            message: truncate(&message, MAX_MESSAGE_CHARS),
            debug: None,
            location: info
                .location()
                .map(|l| format!("{}:{}", l.file(), l.line())),
            backtrace: None,
        }
    }

    /// The record as one JSON line (no trailing newline).
    pub fn to_json_line(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| {
            // Only unpaired surrogates or similar could fail here; keep the
            // fingerprint rather than lose the record.
            format!(
                r#"{{"v":{},"ts":"{}","source":"{}","kind":"{}","fingerprint":"{}","message":"unserializable record"}}"#,
                self.v, self.ts, self.source, self.kind, self.fingerprint
            )
        })
    }
}

// Helpers.
// ----------------------------------------------------------------------------

fn truncate(s: &str, max_chars: usize) -> String {
    match s.char_indices().nth(max_chars) {
        Some((i, _)) => format!("{}…", &s[..i]),
        None => s.to_owned(),
    }
}

fn now_rfc3339() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    rfc3339_from_unix(secs)
}

/// Civil date from days since the epoch (Howard Hinnant's algorithm), so the
/// crate needs no date dependency.
fn rfc3339_from_unix(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

// Tests.
// ----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::{define_internal_error, define_sensitive_error, define_user_error};

    define_internal_error!(SinkInternal, "Sink failed: {what}.", { what: &str });
    define_user_error!(SinkUser, "Nothing to do.");
    define_sensitive_error!(SinkSensitive, "Not allowed.");

    type Seen = Arc<Mutex<Vec<ErrorRecord>>>;

    fn origin() -> ErrorOrigin {
        ErrorOrigin {
            source: "test".into(),
            env: Some("sandbox".into()),
            session: Some("unit".into()),
            version: Some("0.0.0".into()),
        }
    }

    /// Other tests in the crate construct errors too; the one shared sink
    /// records everything and each test asserts only on its own fingerprints.
    fn seen() -> Seen {
        static SEEN: OnceLock<Seen> = OnceLock::new();
        SEEN.get_or_init(|| {
            let seen: Seen = Default::default();
            let sink_seen = seen.clone();
            install_error_sink(Box::new(move |event| {
                sink_seen
                    .lock()
                    .unwrap()
                    .push(ErrorRecord::from_event(&event, &origin()));
            }));
            seen
        })
        .clone()
    }

    fn find(seen: &Seen, suffix: &str) -> ErrorRecord {
        seen.lock()
            .unwrap()
            .iter()
            .rev()
            .find(|r| r.fingerprint.ends_with(suffix))
            .cloned()
            .unwrap_or_else(|| panic!("no record for {suffix}"))
    }

    #[test]
    fn sink_sees_every_constructed_error_with_fingerprint_kind_and_location() {
        let seen = seen();
        let _e1 = SinkInternal::with_debug("io", &"cause");
        let line = line!() - 1;
        let _e2 = SinkUser::new();

        let internal = find(&seen, "::SinkInternal");
        assert_eq!(
            internal.fingerprint,
            "fractic_server_error::sink::tests::SinkInternal"
        );
        assert_eq!(internal.template.as_deref(), Some("Sink failed: {what}."));
        assert_eq!(internal.kind, "internal");
        assert_eq!(internal.message, "Sink failed: io.");
        assert_eq!(internal.debug.as_deref(), Some("\"cause\""));
        assert_eq!(
            internal.location.as_deref(),
            Some(&*format!("{}:{line}", file!()))
        );
        assert_eq!(internal.source, "test");
        assert_eq!(internal.env.as_deref(), Some("sandbox"));
        assert_eq!(find(&seen, "::SinkUser").kind, "user");
    }

    #[test]
    fn sensitive_records_drop_the_debug_cause() {
        let seen = seen();
        let _e = SinkSensitive::with_debug(&"token=abc");
        let sensitive = find(&seen, "::SinkSensitive");
        assert_eq!(sensitive.kind, "sensitive");
        assert_eq!(sensitive.debug, None);
        assert_eq!(sensitive.backtrace, None);
        assert!(sensitive.location.is_some());
    }

    #[test]
    fn records_round_trip_as_one_json_line() {
        let seen = seen();
        let _e = SinkInternal::new("disk");
        let record = find(&seen, "::SinkInternal");
        let line = record.to_json_line();
        assert!(!line.contains('\n'));
        assert_eq!(serde_json::from_str::<ErrorRecord>(&line).unwrap(), record);
        let value: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(value["v"], 1);
        assert_eq!(value["kind"], "internal");
    }

    #[test]
    fn panic_records_carry_the_payload_and_location() {
        let (tx, rx) = std::sync::mpsc::channel();
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let _ = tx.send(ErrorRecord::from_panic(info, &origin()));
        }));
        let result = std::panic::catch_unwind(|| panic!("boom {}", 42));
        std::panic::set_hook(previous);
        assert!(result.is_err());
        let record = rx.recv().unwrap();
        assert_eq!(record.fingerprint, PANIC_FINGERPRINT);
        assert_eq!(record.kind, "critical");
        assert_eq!(record.message, "boom 42");
        assert!(record.location.as_deref().unwrap().starts_with(file!()));
    }

    #[test]
    fn timestamps_are_rfc3339_utc() {
        assert_eq!(rfc3339_from_unix(0), "1970-01-01T00:00:00Z");
        assert_eq!(rfc3339_from_unix(951_782_400), "2000-02-29T00:00:00Z");
        assert_eq!(rfc3339_from_unix(1_794_186_723), "2026-11-09T01:12:03Z");
    }

    #[test]
    fn truncation_respects_char_boundaries() {
        assert_eq!(truncate("héllo", 10), "héllo");
        assert_eq!(truncate("héllo", 2), "hé…");
    }
}
