use colored::Colorize as _;

#[derive(Debug, Clone, PartialEq)]
pub enum ServerErrorBehaviour {
    ForwardToClient,
    LogWarningForwardToClient,
    LogErrorForwardToClient,
    LogWarningSendFixedMsgToClient(&'static str),
    LogErrorSendFixedMsgToClient(&'static str),
    ReturnInternalServerError,
    ReturnUnauthorized,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ServerErrorTag {
    None,
    Critical,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ServerErrorContext {
    /// Don't include any context information. Can be used for sensitive errors,
    /// where we should be careful not to leak implementation details.
    None,
    /// Uses panic::Location::caller(), which is fairly cheap to compute.
    Partial,
    /// If RUST_BACKTRACE is set, uses backtrace::Backtrace::force_capture(),
    /// which is expensive to compute but contains a lot of information.
    /// Otherwise, same as Partial.
    Full,
}

impl ServerErrorContext {
    #[track_caller]
    pub fn capture(&self) -> String {
        match self {
            ServerErrorContext::None => "OMITTED".to_string(),
            ServerErrorContext::Full if std::env::var("RUST_BACKTRACE").is_ok() => {
                std::backtrace::Backtrace::force_capture().to_string()
            }
            _ => {
                let location = std::panic::Location::caller();
                format!("{}; {};", location.file(), location.line())
            }
        }
    }
}

pub trait ServerErrorTrait: std::fmt::Debug + Send + Sync + 'static {
    fn behaviour(&self) -> ServerErrorBehaviour;
    fn tag(&self) -> ServerErrorTag;
    fn context(&self) -> &String;
    fn message(&self) -> &String;
    fn debug(&self) -> Option<&String>;
}

pub type ServerError = Box<dyn ServerErrorTrait>;

impl std::fmt::Display for dyn ServerErrorTrait {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self.tag() {
            ServerErrorTag::None => {}
            ServerErrorTag::Critical => {
                write!(f, "{}", "CRITICAL; ".bold().red())?;
            }
        }
        write!(f, "{}\n{:#?}", self.message().bold(), self)
    }
}

impl std::error::Error for dyn ServerErrorTrait {}

// Tests
// --------------------------------------------------

#[cfg(test)]
mod tests {
    use crate::{
        define_client_error, define_critical_error, define_internal_error, define_sensitive_error,
        define_temporary_error, define_user_error, CLIENT_ERROR_MSG,
    };

    use super::*;

    #[test]
    fn test_internal_error() {
        define_internal_error!(InternalError, "An internal error occurred.");
        let error = InternalError::with_debug(&"debug info".to_string());

        assert_eq!(error.message(), "An internal error occurred.");
        assert!(error.debug().is_some());
        assert!(error.debug().unwrap().contains("debug info"));

        let error_str = format!("{}", error);
        assert!(error_str.contains("An internal error occurred."));
        assert!(error_str.contains("debug info"));

        assert_eq!(
            error.behaviour(),
            ServerErrorBehaviour::ReturnInternalServerError
        );
        assert_eq!(error.tag(), ServerErrorTag::None);
    }

    #[test]
    fn test_critical_error() {
        define_critical_error!(InternalError, "A critical error occurred.");
        let error = InternalError::with_debug(&"debug info".to_string());

        assert_eq!(error.message(), "A critical error occurred.");
        assert!(error.debug().is_some());
        assert!(error.debug().unwrap().contains("debug info"));

        let error_str = format!("{}", error);
        assert!(error_str.contains("CRITICAL"));
        assert!(error_str.contains("A critical error occurred."));
        assert!(error_str.contains("debug info"));

        assert_eq!(
            error.behaviour(),
            ServerErrorBehaviour::ReturnInternalServerError
        );
        assert_eq!(error.tag(), ServerErrorTag::Critical);
    }

    #[test]
    fn test_client_error() {
        define_client_error!(ClientError, "A client error occurred: {code}.", { code: i32 });
        let error = ClientError::new(404);

        assert_eq!(error.message(), "A client error occurred: 404.");
        assert!(error.debug().is_none());

        let error_str = format!("{}", error);
        assert!(error_str.contains("A client error occurred: 404."));
        assert!(!error_str.contains("debug info"));

        assert_eq!(
            error.behaviour(),
            ServerErrorBehaviour::LogErrorSendFixedMsgToClient(CLIENT_ERROR_MSG)
        );
        assert_eq!(error.tag(), ServerErrorTag::None);
    }

    #[test]
    fn test_sensitive_error() {
        define_sensitive_error!(SensitiveError, "Sensitive data error: {details} for user {user}.", { details: &'static str, user: &'static str });
        let error =
            SensitiveError::with_debug("leak detected", "user123", &"debug info".to_string());

        assert_eq!(
            error.message(),
            "Sensitive data error: leak detected for user user123."
        );
        assert!(error.debug().is_some());
        assert!(error.debug().unwrap().contains("debug info"));

        let error_str = format!("{}", error);
        assert!(error_str.contains("Sensitive data error: leak detected for user user123."));
        assert!(error_str.contains("debug info"));

        assert_eq!(error.behaviour(), ServerErrorBehaviour::ReturnUnauthorized);
        assert_eq!(error.tag(), ServerErrorTag::None);
    }

    #[test]
    fn test_user_error() {
        define_user_error!(UserError, "User error occurred.");
        let error = UserError::new();

        assert_eq!(error.message(), "User error occurred.");
        assert!(error.debug().is_none());

        let error_str = format!("{}", error);
        assert!(error_str.contains("User error occurred."));
        assert!(!error_str.contains("debug info"));

        assert_eq!(
            error.behaviour(),
            ServerErrorBehaviour::LogWarningForwardToClient
        );
        assert_eq!(error.tag(), ServerErrorTag::None);
    }

    #[test]
    fn test_temporary_error() {
        define_temporary_error!(TemporaryError, "Temporary issue: {reason}, retry after {seconds} seconds by {method}.", { reason: &'static str, seconds: u32, method: &'static str });
        let error = TemporaryError::new("network outage", 30, "reconnect");

        assert_eq!(
            error.message(),
            "Temporary issue: network outage, retry after 30 seconds by reconnect."
        );
        assert!(error.debug().is_none());

        let error_str = format!("{}", error);
        assert!(error_str
            .contains("Temporary issue: network outage, retry after 30 seconds by reconnect."));
        assert!(!error_str.contains("debug info"));

        assert_eq!(
            error.behaviour(),
            ServerErrorBehaviour::LogWarningForwardToClient
        );
        assert_eq!(error.tag(), ServerErrorTag::None);
    }
}
