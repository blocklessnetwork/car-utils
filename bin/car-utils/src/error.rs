use std::{fmt::Display, process::ExitCode};

use blockless_car::error::CarError;

#[derive(Debug)]
pub(crate) struct UtilError {
    pub(crate) err: String,
    pub(crate) code: u8,
}

impl UtilError {
    pub fn new(err: String) -> Self {
        Self { err, code: 127 }
    }
}

impl Display for UtilError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}. error code {}", self.err, self.code)
    }
}

impl From<std::io::Error> for UtilError {
    fn from(value: std::io::Error) -> Self {
        Self::new(value.to_string())
    }
}

impl From<CarError> for UtilError {
    fn from(value: CarError) -> Self {
        Self::new(value.to_string())
    }
}

impl From<UtilError> for ExitCode {
    fn from(value: UtilError) -> Self {
        ExitCode::from(value.code)
    }
}
