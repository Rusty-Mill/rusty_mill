use std::fmt;

#[derive(Debug)]
pub struct Error {
    msg: String,
    line: usize,
    column: usize,
}

impl Error {
    pub(crate) fn syntax(msg: impl Into<String>, line: usize, column: usize) -> Self {
        Error {
            msg: msg.into(),
            line,
            column,
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        if self.line == 0 {
            write!(f, "{}", self.msg)
        } else {
            write!(
                f,
                "{} at line {} column {}",
                self.msg, self.line, self.column
            )
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
            line: 0,
            column: 0,
        }
    }
}
