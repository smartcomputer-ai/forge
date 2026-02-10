use crate::Diagnostic;
use crate::storage::TurnStoreError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AttractorError {
    #[error("DOT parse error: {0}")]
    DotParse(String),
    #[error("invalid graph: {0}")]
    InvalidGraph(String),
    #[error("stylesheet parse error: {0}")]
    StylesheetParse(String),
    #[error("runtime error: {0}")]
    Runtime(String),
    #[error(transparent)]
    Storage(#[from] TurnStoreError),
    #[error(transparent)]
    Validation(#[from] ValidationError),
}

#[derive(Debug, Error, Clone)]
#[error("validation failed with {errors_count} error(s)")]
pub struct ValidationError {
    pub diagnostics: Vec<Diagnostic>,
    pub errors_count: usize,
}

impl ValidationError {
    pub fn new(diagnostics: Vec<Diagnostic>) -> Self {
        let errors_count = diagnostics.iter().filter(|d| d.is_error()).count();
        Self {
            diagnostics,
            errors_count,
        }
    }
}
