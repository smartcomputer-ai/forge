// Copyright 2025 StrongDM Inc
// SPDX-License-Identifier: Apache-2.0

use std::fmt;

/// CXDB client error type.
#[derive(Debug)]
pub enum Error {
    ClientClosed,
    ContextNotFound,
    TurnNotFound,
    InvalidResponse(String),
    Server(ServerError),
    Io(std::io::Error),
    Tls(String),
    Timeout,
    Cancelled,
    QueueFull,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerError {
    pub code: u32,
    pub detail: String,
}

impl fmt::Display for ServerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "cxdb server error {}: {}", self.code, self.detail)
    }
}

impl std::error::Error for ServerError {}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::ClientClosed => write!(f, "cxdb: client closed"),
            Error::ContextNotFound => write!(f, "cxdb: context not found"),
            Error::TurnNotFound => write!(f, "cxdb: turn not found"),
            Error::InvalidResponse(msg) => write!(f, "cxdb: invalid response: {msg}"),
            Error::Server(err) => write!(f, "{err}"),
            Error::Io(err) => write!(f, "cxdb io: {err}"),
            Error::Tls(err) => write!(f, "cxdb tls: {err}"),
            Error::Timeout => write!(f, "cxdb: deadline exceeded"),
            Error::Cancelled => write!(f, "cxdb: request cancelled"),
            Error::QueueFull => write!(f, "cxdb: request queue full"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io(err) => Some(err),
            Error::Server(err) => Some(err),
            _ => None,
        }
    }
}

impl From<std::io::Error> for Error {
    fn from(err: std::io::Error) -> Self {
        Error::Io(err)
    }
}

pub type Result<T> = std::result::Result<T, Error>;

#[allow(non_upper_case_globals)]
pub const ErrClientClosed: Error = Error::ClientClosed;
#[allow(non_upper_case_globals)]
pub const ErrContextNotFound: Error = Error::ContextNotFound;
#[allow(non_upper_case_globals)]
pub const ErrTurnNotFound: Error = Error::TurnNotFound;
#[allow(non_upper_case_globals)]
pub const ErrInvalidResponse: Error = Error::InvalidResponse(String::new());

/// Checks whether an error is a server error with the specified code.
pub fn is_server_error(err: &Error, code: u32) -> bool {
    matches!(err, Error::Server(ServerError { code: c, .. }) if *c == code)
}

impl Error {
    pub fn invalid_response(msg: impl Into<String>) -> Self {
        Error::InvalidResponse(msg.into())
    }

    pub fn server(code: u32, detail: impl Into<String>) -> Self {
        Error::Server(ServerError {
            code,
            detail: detail.into(),
        })
    }
}
