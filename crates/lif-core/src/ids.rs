//! Object ids.
//!
//! Ids are allocated by the [`Model`](crate::Model) from per-kind counters
//! that start at 0 and are never reused while the server runs, exactly like
//! tmux. They display the way tmux prints them: `$1`, `@2`, `%3`.

use std::fmt;

macro_rules! id_type {
    ($(#[$doc:meta])* $name:ident, $prefix:literal) => {
        $(#[$doc])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(pub u32);

        impl $name {
            /// Parse the printed form (e.g. `"%3"`). Returns `None` for
            /// anything else, including a missing prefix.
            pub fn parse(s: &str) -> Option<Self> {
                let digits = s.strip_prefix($prefix)?;
                if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
                    return None;
                }
                digits.parse().ok().map($name)
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, concat!($prefix, "{}"), self.0)
            }
        }
    };
}

id_type!(
    /// A session. Printed as `$N`.
    SessionId,
    "$"
);
id_type!(
    /// A window. Printed as `@N`.
    WindowId,
    "@"
);
id_type!(
    /// A pane. Printed as `%N`.
    PaneId,
    "%"
);

/// A client connection. Chosen by the server (any value unique among the
/// currently connected clients); the model only stores and compares it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ClientId(pub u32);

impl fmt::Display for ClientId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "client{}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_and_parse() {
        assert_eq!(SessionId(1).to_string(), "$1");
        assert_eq!(WindowId(2).to_string(), "@2");
        assert_eq!(PaneId(3).to_string(), "%3");
        assert_eq!(PaneId::parse("%3"), Some(PaneId(3)));
        assert_eq!(PaneId::parse("%"), None);
        assert_eq!(PaneId::parse("@3"), None);
        assert_eq!(PaneId::parse("%+3"), None);
        assert_eq!(SessionId::parse("$12"), Some(SessionId(12)));
        assert_eq!(WindowId::parse("@x"), None);
    }
}
