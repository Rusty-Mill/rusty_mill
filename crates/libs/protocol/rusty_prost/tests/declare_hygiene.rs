//! The declaration macros must expand correctly in a scope that imports this crate's own
//! one-parameter `Result` alias, which a caller commonly does.

use rusty_prost::{message, open_enum, Result};

open_enum! {
    /// A colour.
    Colour {
        Red = 0: "RED",
        Green = 1: "GREEN",
    }
}

message! {
    /// A paint.
    Paint {
        1 name: String = string,
        2 colour: i32 = enumv,
    }
}

#[test]
fn macros_expand_where_the_result_alias_is_imported() -> Result<()> {
    assert_eq!(Colour::try_from(1), Ok(Colour::Green));
    assert!(Colour::try_from(9).is_err());
    let paint = Paint::default();
    assert_eq!(paint.colour, 0);
    Ok(())
}
