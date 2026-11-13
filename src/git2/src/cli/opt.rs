//! `src/cli/opt.c`: the command-line argument parser of the `git2` CLI (`src/cli/opt.h`),
//! ported to Rust.
//!
//! The header's enums are native Rust: [`Kind`] and [`Status`] are closed enums, whose
//! discriminants stay C's values while the CLI's remaining C, its `cmd_*.c` spec tables,
//! `opt_usage.c` and `common.c`, uses their constants, and [`Usage`] and [`ParseFlags`] are
//! bit sets, which that C combines with `|`. `opt.h` keeps the C definitions for that C:
//! the two are not linked, and nothing here is exported.
#![forbid(unsafe_code)]

use core::ops::{BitAnd, BitAndAssign, BitOr, BitOrAssign};

/// The kind of command-line argument an option spec parses.
///
/// Its discriminants are C's values, the integer form through which the spec tables of the
/// CLI's remaining C pass it, until its `port/layout` removes them.
///
/// Replaces `enm::cli_opt_type_t`
#[doc(alias = "cli_opt_type_t")]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u32)]
pub enum Kind {
    /// No argument: the kind of the zeroed spec that ends a C spec table, which no
    /// argument matches.
    ///
    /// Replaces `enm::CLI_OPT_TYPE_NONE`
    #[doc(alias = "CLI_OPT_TYPE_NONE")]
    None = 0,
    /// An option that sets its value to true when given, as `--debug`, and to false when
    /// given with the implicit `no-` prefix, as `--no-debug`.
    ///
    /// Replaces `enm::CLI_OPT_TYPE_BOOL`
    #[doc(alias = "CLI_OPT_TYPE_BOOL")]
    Bool = 1,
    /// An option that sets its value to the spec's switch value when given, with no implicit
    /// negation: several switches may share one value to select a mode, as `--read` and
    /// `--write`.
    ///
    /// Replaces `enm::CLI_OPT_TYPE_SWITCH`
    #[doc(alias = "CLI_OPT_TYPE_SWITCH")]
    Switch = 2,
    /// An option that adds the spec's switch value to its value each time it is given, as
    /// `-vvv` for a verbosity of 3.
    ///
    /// Replaces `enm::CLI_OPT_TYPE_ACCUMULATOR`
    #[doc(alias = "CLI_OPT_TYPE_ACCUMULATOR")]
    Accumulator = 3,
    /// An option that takes a value: `-n value`, `-nvalue`, `--name value` or
    /// `--name=value`.
    ///
    /// Replaces `enm::CLI_OPT_TYPE_VALUE`
    #[doc(alias = "CLI_OPT_TYPE_VALUE")]
    Value = 4,
    /// The bare `--` after which every argument is literal, even one that looks like an
    /// option, as the `-rf` of `program -- -rf`.
    ///
    /// Replaces `enm::CLI_OPT_TYPE_LITERAL`
    #[doc(alias = "CLI_OPT_TYPE_LITERAL")]
    Literal = 5,
    /// A single argument that is not an option: once the options are exhausted, bare
    /// arguments match the argument specs in the order they are specified.
    ///
    /// Replaces `enm::CLI_OPT_TYPE_ARG`
    #[doc(alias = "CLI_OPT_TYPE_ARG")]
    Arg = 6,
    /// The remaining arguments, as a list, such as several paths.
    ///
    /// Replaces `enm::CLI_OPT_TYPE_ARGS`
    #[doc(alias = "CLI_OPT_TYPE_ARGS")]
    Args = 7,
}

impl Kind {
    /// The kind whose C value is `raw`, or `None` for a value `opt.h` does not name.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "the integer form through which opt.c's export gateways, written by its \
                      later batches, take a spec's kind from C"
        )
    )]
    pub(crate) const fn from_raw(raw: u32) -> Option<Self> {
        match raw {
            0 => Some(Self::None),
            1 => Some(Self::Bool),
            2 => Some(Self::Switch),
            3 => Some(Self::Accumulator),
            4 => Some(Self::Value),
            5 => Some(Self::Literal),
            6 => Some(Self::Arg),
            7 => Some(Self::Args),
            _ => None,
        }
    }

    /// The C value of the kind.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "the integer form through which opt.c's export gateways, written by its \
                      later batches, give a spec's kind to C"
        )
    )]
    pub(crate) const fn raw(self) -> u32 {
        self as u32
    }
}

/// How an option spec is parsed and shown in usage: a set of flags, any combination of
/// which is valid.
///
/// The bits of [`from_bits_retain`](Self::from_bits_retain) that no constant names are kept,
/// as C keeps them in a spec's `usage`.
///
/// Replaces `enm::cli_opt_usage_t`
#[doc(alias = "cli_opt_usage_t")]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct Usage(u32);

impl Usage {
    /// The argument is a choice with the spec before it: when that spec is `-f` and this one
    /// `-b`, one of `-f` or `-b` may be given.
    ///
    /// Replaces `enm::CLI_OPT_USAGE_CHOICE`
    #[doc(alias = "CLI_OPT_USAGE_CHOICE")]
    pub const CHOICE: Self = Self(1 << 1);
    /// No flag: the spec parses and shows in usage by default.
    ///
    /// Replaces `enm::CLI_OPT_USAGE_DEFAULT`
    #[doc(alias = "CLI_OPT_USAGE_DEFAULT")]
    pub const DEFAULT: Self = Self(0);
    /// The argument is not shown in usage.
    ///
    /// Replaces `enm::CLI_OPT_USAGE_HIDDEN`
    #[doc(alias = "CLI_OPT_USAGE_HIDDEN")]
    pub const HIDDEN: Self = Self(1 << 4);
    /// The argument is required.
    ///
    /// Replaces `enm::CLI_OPT_USAGE_REQUIRED`
    #[doc(alias = "CLI_OPT_USAGE_REQUIRED")]
    pub const REQUIRED: Self = Self(1 << 0);
    /// Usage shows the argument's long form instead of its short alias.
    ///
    /// Replaces `enm::CLI_OPT_USAGE_SHOW_LONG`
    #[doc(alias = "CLI_OPT_USAGE_SHOW_LONG")]
    pub const SHOW_LONG: Self = Self(1 << 5);
    /// The argument ends parsing, as `--help` does.
    ///
    /// Replaces `enm::CLI_OPT_USAGE_STOP_PARSING`
    #[doc(alias = "CLI_OPT_USAGE_STOP_PARSING")]
    pub const STOP_PARSING: Self = Self(1 << 2);
    /// The argument's value is optional: `-n` or `-n foo`.
    ///
    /// Replaces `enm::CLI_OPT_USAGE_VALUE_OPTIONAL`
    #[doc(alias = "CLI_OPT_USAGE_VALUE_OPTIONAL")]
    pub const VALUE_OPTIONAL: Self = Self(1 << 3);

    /// The set of `bits`, each kept, those no constant names included.
    #[must_use]
    pub const fn from_bits_retain(bits: u32) -> Self {
        Self(bits)
    }

    /// The set's bits, as a spec's `usage` holds them.
    #[must_use]
    pub const fn bits(self) -> u32 {
        self.0
    }

    /// Whether every flag of `other` is set.
    #[must_use]
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    /// Whether any flag of `other` is set.
    #[must_use]
    pub const fn intersects(self, other: Self) -> bool {
        self.0 & other.0 != 0
    }

    /// Whether no flag is set.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }
}

impl BitOr for Usage {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

impl BitOrAssign for Usage {
    fn bitor_assign(&mut self, rhs: Self) {
        self.0 |= rhs.0;
    }
}

impl BitAnd for Usage {
    type Output = Self;

    fn bitand(self, rhs: Self) -> Self {
        Self(self.0 & rhs.0)
    }
}

impl BitAndAssign for Usage {
    fn bitand_assign(&mut self, rhs: Self) {
        self.0 &= rhs.0;
    }
}

/// How a parser treats its arguments: a set of flags, any combination of which is valid.
///
/// The bits of [`from_bits_retain`](Self::from_bits_retain) that no constant names are kept,
/// as C keeps them in a parser's `flags`.
///
/// Replaces `enm::cli_opt_flag_t`
#[doc(alias = "cli_opt_flag_t")]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct ParseFlags(u32);

impl ParseFlags {
    /// No flag: the arguments are parsed in the order they are given, never reordered.
    ///
    /// Replaces `enm::CLI_OPT_PARSE_DEFAULT`
    #[doc(alias = "CLI_OPT_PARSE_DEFAULT")]
    pub const DEFAULT: Self = Self(0);
    /// GNU `getopt_long` style whatever the environment: `POSIXLY_CORRECT` is ignored.
    ///
    /// Replaces `enm::CLI_OPT_PARSE_FORCE_GNU`
    #[doc(alias = "CLI_OPT_PARSE_FORCE_GNU")]
    pub const FORCE_GNU: Self = Self(1 << 1);
    /// GNU `getopt_long` style: options and arguments may be intermixed, as
    /// `file1 --help file2`, unless the `POSIXLY_CORRECT` environment variable is set. Like
    /// `getopt_long`, the parser may then reorder the arguments it is given.
    ///
    /// Replaces `enm::CLI_OPT_PARSE_GNU`
    #[doc(alias = "CLI_OPT_PARSE_GNU")]
    pub const GNU: Self = Self(1 << 0);

    /// The set of `bits`, each kept, those no constant names included.
    #[must_use]
    pub const fn from_bits_retain(bits: u32) -> Self {
        Self(bits)
    }

    /// The set's bits, as a parser's `flags` holds them.
    #[must_use]
    pub const fn bits(self) -> u32 {
        self.0
    }

    /// Whether every flag of `other` is set.
    #[must_use]
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    /// Whether any flag of `other` is set.
    #[must_use]
    pub const fn intersects(self, other: Self) -> bool {
        self.0 & other.0 != 0
    }

    /// Whether no flag is set.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }
}

impl BitOr for ParseFlags {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

impl BitOrAssign for ParseFlags {
    fn bitor_assign(&mut self, rhs: Self) {
        self.0 |= rhs.0;
    }
}

impl BitAnd for ParseFlags {
    type Output = Self;

    fn bitand(self, rhs: Self) -> Self {
        Self(self.0 & rhs.0)
    }
}

impl BitAndAssign for ParseFlags {
    fn bitand_assign(&mut self, rhs: Self) {
        self.0 &= rhs.0;
    }
}

/// The outcome of parsing one command-line argument.
///
/// Its discriminants are C's values, the integer form through which the CLI's remaining C
/// reads a parse's outcome, until its `port/layout` removes them: that C tests `Done`, `0`,
/// as false and every other outcome as true.
///
/// Replaces `enm::cli_opt_status_t`
#[doc(alias = "cli_opt_status_t")]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u32)]
pub enum Status {
    /// Parsing is complete: no argument is left.
    ///
    /// Replaces `enm::CLI_OPT_STATUS_DONE`
    #[doc(alias = "CLI_OPT_STATUS_DONE")]
    Done = 0,
    /// The argument was parsed: the option it matched is known, and its value set.
    ///
    /// Replaces `enm::CLI_OPT_STATUS_OK`
    #[doc(alias = "CLI_OPT_STATUS_OK")]
    Ok = 1,
    /// The argument matches no spec.
    ///
    /// Replaces `enm::CLI_OPT_STATUS_UNKNOWN_OPTION`
    #[doc(alias = "CLI_OPT_STATUS_UNKNOWN_OPTION")]
    UnknownOption = 2,
    /// The argument matched a spec of kind [`Kind::Value`] whose value is not optional, and
    /// no value was given.
    ///
    /// Replaces `enm::CLI_OPT_STATUS_MISSING_VALUE`
    #[doc(alias = "CLI_OPT_STATUS_MISSING_VALUE")]
    MissingValue = 3,
    /// A required argument was not given.
    ///
    /// Replaces `enm::CLI_OPT_STATUS_MISSING_ARGUMENT`
    #[doc(alias = "CLI_OPT_STATUS_MISSING_ARGUMENT")]
    MissingArgument = 4,
}

impl Status {
    /// The outcome whose C value is `raw`, or `None` for a value `opt.h` does not name.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "the integer form through which opt.c's export gateways, written by its \
                      later batches, take an outcome from C"
        )
    )]
    pub(crate) const fn from_raw(raw: u32) -> Option<Self> {
        match raw {
            0 => Some(Self::Done),
            1 => Some(Self::Ok),
            2 => Some(Self::UnknownOption),
            3 => Some(Self::MissingValue),
            4 => Some(Self::MissingArgument),
            _ => None,
        }
    }

    /// The C value of the outcome.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "the integer form through which opt.c's export gateways, written by its \
                      later batches, give an outcome to C"
        )
    )]
    pub(crate) const fn raw(self) -> u32 {
        self as u32
    }
}

/// The integer forms of [`Kind`] and [`Status`], compared with the constants `opt.h` defines,
/// through the raw bindings, which only constants reach: no C runs.
#[cfg(test)]
mod equiv_priv {
    use super::{Kind, Status};
    use crate::sys::git2 as sys;

    /// Every kind, paired with its C constant.
    const KINDS: [(Kind, u32); 8] = [
        (Kind::None, sys::CLI_OPT_TYPE_NONE),
        (Kind::Bool, sys::CLI_OPT_TYPE_BOOL),
        (Kind::Switch, sys::CLI_OPT_TYPE_SWITCH),
        (Kind::Accumulator, sys::CLI_OPT_TYPE_ACCUMULATOR),
        (Kind::Value, sys::CLI_OPT_TYPE_VALUE),
        (Kind::Literal, sys::CLI_OPT_TYPE_LITERAL),
        (Kind::Arg, sys::CLI_OPT_TYPE_ARG),
        (Kind::Args, sys::CLI_OPT_TYPE_ARGS),
    ];

    /// Every outcome, paired with its C constant.
    const STATUSES: [(Status, u32); 5] = [
        (Status::Done, sys::CLI_OPT_STATUS_DONE),
        (Status::Ok, sys::CLI_OPT_STATUS_OK),
        (Status::UnknownOption, sys::CLI_OPT_STATUS_UNKNOWN_OPTION),
        (Status::MissingValue, sys::CLI_OPT_STATUS_MISSING_VALUE),
        (
            Status::MissingArgument,
            sys::CLI_OPT_STATUS_MISSING_ARGUMENT,
        ),
    ];

    /// Each kind converts to its C constant and back.
    #[test]
    fn kinds_round_trip_through_their_c_values() {
        for (kind, c) in KINDS {
            assert_eq!(kind.raw(), c, "{kind:?}");
            assert_eq!(Kind::from_raw(c), Some(kind), "{c}");
        }
    }

    /// A value `opt.h` does not name is no kind: past the last constant, and at the top of
    /// the range.
    #[test]
    fn values_past_the_c_kinds_are_none() {
        assert_eq!(Kind::from_raw(sys::CLI_OPT_TYPE_ARGS + 1), None);
        assert_eq!(Kind::from_raw(u32::MAX), None);
    }

    /// Each outcome converts to its C constant and back.
    #[test]
    fn statuses_round_trip_through_their_c_values() {
        for (status, c) in STATUSES {
            assert_eq!(status.raw(), c, "{status:?}");
            assert_eq!(Status::from_raw(c), Some(status), "{c}");
        }
    }

    /// A value `opt.h` does not name is no outcome: past the last constant, and at the top of
    /// the range.
    #[test]
    fn values_past_the_c_statuses_are_none() {
        assert_eq!(
            Status::from_raw(sys::CLI_OPT_STATUS_MISSING_ARGUMENT + 1),
            None
        );
        assert_eq!(Status::from_raw(u32::MAX), None);
    }
}
