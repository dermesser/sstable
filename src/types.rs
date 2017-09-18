#![allow(dead_code)]

//! A collection of fundamental and/or simple types used by other modules

use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::io;
use std::result;

#[derive(Debug, PartialOrd, PartialEq)]
pub enum ValueType {
    TypeDeletion = 0,
    TypeValue = 1,
}

/// Represents a sequence number of a single entry.
pub type SequenceNumber = u64;

pub const MAX_SEQUENCE_NUMBER: SequenceNumber = (1 << 56) - 1;

#[derive(Clone, Debug)]
#[allow(dead_code)]
pub enum Status {
    OK,
    NotFound(String),
    Corruption(String),
    NotSupported(String),
    InvalidArgument(String),
    PermissionDenied(String),
    IOError(String),
    Unknown(String),
}

impl Display for Status {
    fn fmt(&self, fmt: &mut Formatter) -> result::Result<(), fmt::Error> {
        fmt.write_str(self.description())
    }
}

impl Error for Status {
    fn description(&self) -> &str {
        match *self {
            Status::OK => "ok",
            Status::NotFound(ref s) => s,
            Status::Corruption(ref s) => s,
            Status::NotSupported(ref s) => s,
            Status::InvalidArgument(ref s) => s,
            Status::PermissionDenied(ref s) => s,
            Status::IOError(ref s) => s,
            Status::Unknown(ref s) => s,
        }
    }
}

/// LevelDB's result type
pub type Result<T> = result::Result<T, Status>;

pub fn from_io_result<T>(e: io::Result<T>) -> Result<T> {
    match e {
        Ok(r) => result::Result::Ok(r),
        Err(e) => {
            let err = e.description().to_string();

            let r = match e.kind() {
                io::ErrorKind::NotFound => Err(Status::NotFound(err)),
                io::ErrorKind::InvalidData => Err(Status::Corruption(err)),
                io::ErrorKind::InvalidInput => Err(Status::InvalidArgument(err)),
                io::ErrorKind::PermissionDenied => Err(Status::PermissionDenied(err)),
                _ => Err(Status::IOError(err)),
            };

            r
        }
    }
}
