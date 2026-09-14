//! Structured error type + exit-code mapping (AXI convention: 0 success / 1 operational / 2 usage).

use std::fmt;

#[derive(Debug)]
pub struct Error {
    pub message: String,
    pub code: &'static str,
    /// Usage vs operational family. Usage-family errors exit 2 (a wrong command/input, the
    /// caller's mistake) even when they carry a *precise* code; operational errors exit 1.
    pub usage: bool,
    pub suggestions: Vec<String>,
    /// Help text printed after the error (usage mistakes: a typo'd flag or command).
    pub help: Option<String>,
}

impl Error {
    pub fn operational(message: impl Into<String>, code: &'static str) -> Self {
        Error {
            message: message.into(),
            code,
            usage: false,
            suggestions: Vec::new(),
            help: None,
        }
    }

    pub fn usage(message: impl Into<String>) -> Self {
        Error {
            message: message.into(),
            code: "VALIDATION_ERROR",
            usage: true,
            suggestions: Vec::new(),
            help: None,
        }
    }

    /// A usage-family error with a precise (non-`VALIDATION_ERROR`) code, e.g. `unknown-kind`.
    /// Still exits 2 — the input is the caller's mistake — but the code is precise so callers
    /// and tests can match on the specific refusal instead of the generic validation bucket.
    pub fn usage_code(message: impl Into<String>, code: &'static str) -> Self {
        Error {
            message: message.into(),
            code,
            usage: true,
            suggestions: Vec::new(),
            help: None,
        }
    }

    pub fn with_help(mut self, help: String) -> Self {
        self.help = Some(help);
        self
    }

    pub fn with_suggestions(mut self, suggestions: Vec<String>) -> Self {
        self.suggestions = suggestions;
        self
    }

    pub fn exit_code(&self) -> i32 {
        if self.usage {
            2
        } else {
            1
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;
