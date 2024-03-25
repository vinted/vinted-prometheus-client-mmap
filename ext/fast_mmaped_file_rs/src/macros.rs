#[macro_export]
macro_rules! err {
    (with_errno: $err_t:expr, $($arg:expr),*) => {
        {
            let err = format!($($arg),*);
            let strerror = strerror(errno());
            Error::new($err_t, format!("{err} ({strerror})"))
        }
    };

    ($err_t:expr, $($arg:expr),*) => {
        Error::new($err_t, format!($($arg),*))
    };
}
