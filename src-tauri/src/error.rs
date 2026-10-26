use serde::{Serialize, Serializer};

/// Error type returned by every Tauri command. Wraps `anyhow` so internal
/// code can use `?` freely, and serializes to a plain message for the UI.
#[derive(Debug)]
pub struct Error(anyhow::Error);

impl<E: Into<anyhow::Error>> From<E> for Error {
    fn from(err: E) -> Self {
        Self(err.into())
    }
}

impl Serialize for Error {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&format!("{:#}", self.0))
    }
}

pub type CmdResult<T> = Result<T, Error>;

/// Like `anyhow::ensure!`, but also usable in functions returning `CmdResult`.
macro_rules! ensure {
    ($cond:expr, $($arg:tt)+) => {
        if !$cond {
            return Err(anyhow::anyhow!($($arg)+).into());
        }
    };
}
pub(crate) use ensure;
