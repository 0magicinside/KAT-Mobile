// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use std::{error, fmt, result};

/// A result of a function that may return an [`RtlsdrError`].
pub type Result<T> = result::Result<T, RtlsdrError>;

#[derive(Debug)]
pub enum RtlsdrError {
    Usb(String),
    Timeout,
    RtlsdrErr(String),
    /// No tuner this library has a driver for answered on the dongle's I2C bus.
    UnknownTuner,
}

impl RtlsdrError {
    /// Returns true when the underlying USB transfer timed out.
    pub fn is_timeout(&self) -> bool {
        matches!(self, RtlsdrError::Timeout)
    }
}

impl fmt::Display for RtlsdrError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RtlsdrError::Usb(err) => f.write_str(err),
            RtlsdrError::Timeout => f.write_str("USB transfer timed out"),
            RtlsdrError::RtlsdrErr(err) => f.write_str(err),
            RtlsdrError::UnknownTuner => {
                f.write_str("No supported tuner found (R820T, R820T2, or R828D required)")
            }
        }
    }
}

impl From<String> for RtlsdrError {
    fn from(e: String) -> Self {
        RtlsdrError::RtlsdrErr(e)
    }
}

impl error::Error for RtlsdrError {}
