use std::fmt;
use std::panic::Location;

macro_rules! error {
    ($($argument:tt)*) => {
        $crate::error::Error::message(format_args!($($argument)*))
    };
}

pub(crate) use error;

pub struct Error {
    message: Box<str>,
    context: Box<str>,
    location: &'static Location<'static>,
}

impl Error {
    #[track_caller]
    pub fn message(message: impl fmt::Display) -> Self {
        Self {
            message: message.to_string().into_boxed_str(),
            context: Box::default(),
            location: Location::caller(),
        }
    }
}

impl fmt::Debug for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, formatter)
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.message)?;
        if !self.context.is_empty() {
            write!(formatter, ": {}", self.context)?;
        }
        write!(
            formatter,
            " [{}:{}:{}]",
            self.location.file(),
            self.location.line(),
            self.location.column()
        )
    }
}

pub trait Context<T> {
    fn context(self, message: impl fmt::Display) -> crate::Result<T>;
}

impl<T, E: fmt::Display> Context<T> for std::result::Result<T, E> {
    #[track_caller]
    fn context(self, message: impl fmt::Display) -> crate::Result<T> {
        match self {
            Ok(value) => Ok(value),
            Err(context) => Err(Error {
                message: message.to_string().into_boxed_str(),
                context: context.to_string().into_boxed_str(),
                location: Location::caller(),
            }),
        }
    }
}

impl<T> Context<T> for Option<T> {
    #[track_caller]
    fn context(self, message: impl fmt::Display) -> crate::Result<T> {
        match self {
            Some(value) => Ok(value),
            None => Err(Error::message(message)),
        }
    }
}
