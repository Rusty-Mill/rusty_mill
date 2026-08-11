use std::fmt;

#[derive(Debug)]
pub struct Error {
    msg: String,
    offset: Option<usize>,
}

impl Error {
    pub(crate) fn syntax(msg: impl Into<String>, offset: usize) -> Self {
        Error {
            msg: msg.into(),
            offset: Some(offset),
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self.offset {
            Some(offset) => write!(f, "{} at byte {}", self.msg, offset),
            None => write!(f, "{}", self.msg),
        }
    }
}

impl std::error::Error for Error {}

impl crate::error::Error for Error {
    fn custom<T>(msg: T) -> Self
    where
        T: fmt::Display,
    {
        Error {
            msg: msg.to_string(),
            offset: None,
        }
    }
}
