//! Package implement editing tool-kit for terminal based apps.

#![feature(box_syntax, box_patterns)]
use std::{fmt, result};

#[macro_use]
pub mod util;
pub mod buffer;
pub mod config;
pub mod event;
mod ftype;
pub mod ftype_txt_en;
mod keymap;
mod keymap_ted;
pub mod location;
#[macro_use]
pub mod window;
pub mod cmd;
mod col_nu;
pub mod state;
mod view;
pub mod window_code;
pub mod window_edit;
pub mod window_file;
pub mod window_line;
pub mod window_prompt;

pub type Result<T> = result::Result<T, Error>;

pub enum Error {
    Fatal(String),
    BadPattern(String),
    IOError(String),
    FailConvert(String),
    FailBuffer(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> result::Result<(), fmt::Error> {
        match self {
            Error::Fatal(msg) => write!(f, "Fatal: {}", msg),
            Error::BadPattern(msg) => write!(f, "BadPattern: {}", msg),
            Error::IOError(msg) => write!(f, "IOError: {}", msg),
            Error::FailConvert(msg) => write!(f, "FailConvert: {}", msg),
            Error::FailBuffer(msg) => write!(f, "FailBuffer: {}", msg),
        }
    }
}

impl fmt::Debug for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> result::Result<(), fmt::Error> {
        write!(f, "{}", self)
    }
}
