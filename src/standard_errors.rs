use crate::define_critical_error;

// Define a couple general-purpose error types.
// --------------------------------------------------

// For unexpected, unrecoverable errors:
define_critical_error!(CriticalError, "Unexpected: {details}.", { details: &str });

// When process fails to spawn child threads:
define_critical_error!(MultithreadingError, "Error executing child threads.");

// Math errors:
define_critical_error!(DivisionByZeroError, "Division by zero.");

// Generic initialization errors:
define_critical_error!(InitError, "Initialization failed: {details}.", { details: &str });

// Logic is not implemented:
define_critical_error!(NotImplementedError, "Logic is not yet implemented.");
