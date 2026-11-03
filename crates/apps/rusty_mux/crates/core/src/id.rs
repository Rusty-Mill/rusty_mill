//! Typed identifiers, so a block id can never be passed where a window id is expected.

use std::fmt;

use rusty_uuid::Uuid;

macro_rules! id {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub struct $name(Uuid);

        impl $name {
            pub(crate) fn random() -> Self {
                Self(Uuid::new_v4())
            }

            /// Wraps an id received from the wire.
            pub const fn from_uuid(uuid: Uuid) -> Self {
                Self(uuid)
            }

            /// The underlying UUID, for the wire.
            pub const fn uuid(self) -> Uuid {
                self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(f)
            }
        }
    };
}

id!(
    /// Identifies a [`Session`](crate::Session).
    SessionId
);
id!(
    /// Identifies a [`Window`](crate::Window).
    WindowId
);
id!(
    /// Identifies a [`Block`](crate::Block): one PTY.
    BlockId
);
id!(
    /// Identifies a [`Client`](crate::Client): one connection.
    ClientId
);

/// The id of an entity of any kind, for operations that apply to all four,
/// such as renaming.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EntityId {
    Session(SessionId),
    Window(WindowId),
    Block(BlockId),
    Client(ClientId),
}
