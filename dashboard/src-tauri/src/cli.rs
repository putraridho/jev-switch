//! The command line: `--hidden` and nothing else (DESIGN 9.13).

/// What the command line asks for.
#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub struct Args {
    /// Start without a window (the autostart entry passes `--hidden`).
    pub hidden: bool,
}

/// `--hidden` anywhere sets `hidden`; unknown arguments are ignored.
pub fn parse<I: IntoIterator<Item = String>>(args: I) -> Args {
    Args {
        hidden: args.into_iter().any(|arg| arg == "--hidden"),
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::as_conversions,
    clippy::disallowed_types,
    clippy::disallowed_macros
)]
mod tests {
    use super::*;

    fn parsed(args: &[&str]) -> Args {
        parse(args.iter().map(|s| (*s).to_owned()))
    }

    #[test]
    fn hidden_anywhere_and_nothing_else_matters() {
        assert_eq!(parsed(&[]), Args { hidden: false });
        assert_eq!(parsed(&["--hidden"]), Args { hidden: true });
        assert_eq!(
            parsed(&["--verbose", "x", "--hidden", "y"]),
            Args { hidden: true }
        );
        assert_eq!(
            parsed(&["--Hidden", "-hidden", "hidden", "--hidden=1"]),
            Args { hidden: false }
        );
    }
}
