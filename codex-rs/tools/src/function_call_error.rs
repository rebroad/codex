use thiserror::Error;

/// Error returned while executing a model-visible tool invocation.
#[derive(Debug, Error, PartialEq)]
pub enum FunctionCallError {
    #[error("failed to parse function arguments: {0}")]
    MalformedArguments(String),
    #[error("{0}")]
    RespondToModel(String),
    #[error("Fatal error: {0}")]
    Fatal(String),
}

impl FunctionCallError {
    pub fn failure_kind(&self) -> Option<&'static str> {
        match self {
            Self::MalformedArguments(_) => Some("parse_arguments"),
            Self::RespondToModel(_) | Self::Fatal(_) => None,
        }
    }
}

#[cfg(test)]
#[path = "function_call_error_tests.rs"]
mod tests;
