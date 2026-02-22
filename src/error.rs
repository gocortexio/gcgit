// SPDX-FileCopyrightText: GoCortexIO
// SPDX-License-Identifier: AGPL-3.0-or-later

use anyhow::Result;
use std::fmt;

#[derive(Debug)]
#[allow(clippy::enum_variant_names)]
pub enum GcgitError {
    GitError(String),
    ConfigError(String),
    ApiError(String),
    ParseError(String),
    #[allow(dead_code)]
    ValidationError(String),
    FileSystemError(String),
}

impl fmt::Display for GcgitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GcgitError::GitError(msg) => write!(f, "Git error: {msg}"),
            GcgitError::ConfigError(msg) => write!(f, "Configuration error: {msg}"),
            GcgitError::ApiError(msg) => write!(f, "API error: {msg}"),
            GcgitError::ParseError(msg) => write!(f, "Parse error: {msg}"),
            GcgitError::ValidationError(msg) => write!(f, "Validation error: {msg}"),
            GcgitError::FileSystemError(msg) => write!(f, "File system error: {msg}"),
        }
    }
}

impl std::error::Error for GcgitError {}

#[allow(dead_code)]
pub type GcgitResult<T> = Result<T, GcgitError>;

impl From<git2::Error> for GcgitError {
    fn from(err: git2::Error) -> Self {
        GcgitError::GitError(err.to_string())
    }
}

impl From<reqwest::Error> for GcgitError {
    fn from(err: reqwest::Error) -> Self {
        GcgitError::ApiError(err.to_string())
    }
}

impl From<serde_yaml_ng::Error> for GcgitError {
    fn from(err: serde_yaml_ng::Error) -> Self {
        GcgitError::ParseError(err.to_string())
    }
}

impl From<std::io::Error> for GcgitError {
    fn from(err: std::io::Error) -> Self {
        GcgitError::FileSystemError(err.to_string())
    }
}

impl From<toml::de::Error> for GcgitError {
    fn from(err: toml::de::Error) -> Self {
        GcgitError::ConfigError(err.to_string())
    }
}
