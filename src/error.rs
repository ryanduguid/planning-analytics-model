//! The failures the engine reports, and the exit codes the command line maps them to.

/// 0: the command finished and found nothing wrong.
pub const EXIT_OK: i32 = 0;
/// 1: a usage or input error.
pub const EXIT_USAGE: i32 = 1;
/// 2: the model is not sound.
pub const EXIT_INVALID_MODEL: i32 = 2;
/// 3: a calculation failed.
pub const EXIT_CALCULATION: i32 = 3;

/// An exception the Python engine did not handle, which ended in a traceback
/// and exit 1. The binary reports it on one line instead.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Uncaught {
    /// The exception's class name, as the traceback's last line showed it.
    pub name: String,
    pub message: String,
}

impl Uncaught {
    pub fn new(name: &str, message: impl Into<String>) -> Uncaught {
        Uncaught {
            name: name.into(),
            message: message.into(),
        }
    }

    /// A trapped `decimal` signal, as the traceback named it.
    pub fn decimal(signal: crate::decimal::Signal) -> Uncaught {
        Uncaught::new(
            &format!("decimal.{}", signal.name()),
            format!("[<class 'decimal.{}'>]", signal.name()),
        )
    }
}

/// A failure while loading, validating or calculating.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// Python's `ModelError`: model source missing, malformed or self contradictory.
    Model(String),
    /// Python's `EvaluationError`: a rule could not be evaluated.
    Evaluation(String),
    /// Python's `RuleSyntaxError`.
    RuleSyntax(String),
    /// A `UnicodeDecodeError` while reading model or input text.
    Decode(String),
    /// An `OSError`, already rendered as Python's `str()` of it.
    Os(String),
    /// The command line's own refusal, carrying its exit code.
    Cli(i32, String),
    Uncaught(Uncaught),
}

impl Error {
    pub fn model(message: impl Into<String>) -> Error {
        Error::Model(message.into())
    }

    pub fn evaluation(message: impl Into<String>) -> Error {
        Error::Evaluation(message.into())
    }

    pub fn cli(code: i32, message: impl Into<String>) -> Error {
        Error::Cli(code, message.into())
    }

    /// `str(error)`.
    pub fn message(&self) -> String {
        match self {
            Error::Model(m)
            | Error::Evaluation(m)
            | Error::RuleSyntax(m)
            | Error::Decode(m)
            | Error::Os(m)
            | Error::Cli(_, m) => m.clone(),
            Error::Uncaught(u) => u.message.clone(),
        }
    }

    /// `isinstance(error, ModelError)`, which EvaluationError also satisfies.
    pub fn is_model_error(&self) -> bool {
        matches!(self, Error::Model(_) | Error::Evaluation(_))
    }
}

impl From<Uncaught> for Error {
    fn from(error: Uncaught) -> Error {
        Error::Uncaught(error)
    }
}

pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_is_str_of_the_error() {
        for (error, text) in [
            (Error::model("model"), "model"),
            (Error::evaluation("evaluation"), "evaluation"),
            (Error::RuleSyntax("syntax".into()), "syntax"),
            (Error::Decode("decode".into()), "decode"),
            (Error::Os("os".into()), "os"),
            (Error::cli(EXIT_USAGE, "cli"), "cli"),
            (
                Error::from(Uncaught::new("ValueError", "uncaught")),
                "uncaught",
            ),
        ] {
            assert_eq!(error.message(), text);
        }
    }
}
