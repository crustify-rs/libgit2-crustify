//! `src/cli/opt.c`: the command-line argument parser of the `git2` CLI (`src/cli/opt.h`),
//! ported to Rust.
//!
//! The header's enums are native Rust: [`Kind`] and [`Status`] are closed enums, whose
//! discriminants stay C's values while the CLI's remaining C, its `cmd_*.c` spec tables,
//! `opt_usage.c` and `common.c`, uses their constants, and [`Usage`] and [`ParseFlags`] are
//! bit sets, which that C combines with `|`. `opt.h` keeps the C definitions for that C:
//! the two are not linked, and nothing here is exported.
//!
//! The parser's helpers are native Rust too: [`Kind::is_option`], `opt.c`'s
//! `spec_is_option_type`, and [`uses_gnu_style`], its `support_gnu_style`: no C outside
//! `opt.c` reaches either, so neither is exported.
//!
//! # The parser's structs
//!
//! The header's three structs keep their C layouts, which that C builds and reads in place:
//! its commands define their spec tables as `const cli_opt_spec[]` arrays, ended by a zeroed
//! spec, with each spec's `value` pointing at the command's own variable, and hold a
//! `cli_opt` and, in `main.c`, a `cli_opt_parser` on their stacks. Rust reaches them as
//! layouts with borrowed handles, until their `port/layout` stages give them native ones:
//!
//! - [`Spec`] (`cli_opt_spec`) describes one option or argument: its [`Kind`] with the variable a
//!   parse writes, as one [`Target`], its long name, its one-character alias, its switch value, its
//!   [`Usage`], and the value name and help text usage shows. A [`SpecTable`] is a run of specs
//!   ended by a zeroed one, as C passes them.
//! - [`Opt`] (`cli_opt`) is the outcome of parsing one argument: its [`Status`], the spec it
//!   matched, the argument and its value as they were given, and how many arguments an
//!   [`Kind::Args`] spec took.
//! - [`Parser`] (`cli_opt_parser`) is the parser's state over a spec table and an argument array:
//!   its [`ParseFlags`], its position, and whether it still has to reorder the arguments GNU style.
//!   [`Parser::new`] sets one up.
//!
//! Nothing here is owned: a parser, an outcome and a spec borrow their spec table, their
//! arguments and their spec's variables, which their caller keeps, and C never frees any of
//! them. Each of the three carries that borrow as `'p`, in which it is invariant, so that a
//! parse only stores into a variable, or hands out an argument, that lives as long as
//! everything that may read it. An argument is an [`Arg`], a borrowed C string; a variable a
//! parse writes is a [`Cell`](core::cell::Cell), or an [`ArgsSlot`] for the remaining
//! arguments, since several specs may share one, as switches multiplexing one mode do. C
//! parses on the thread that holds them, so none of them is `Send` or `Sync`.

use core::cell::Cell;
use core::ffi::{CStr, c_int};
use core::fmt;
use core::num::NonZero;
use core::ops::{BitAnd, BitAndAssign, BitOr, BitOrAssign};

pub use ffi_import::{
    Arg, Opt, OptMut, OptRef, Parser, ParserMut, ParserRef, Spec, SpecMut, SpecRef, SpecTable,
};

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
    pub(crate) const fn raw(self) -> u32 {
        self as u32
    }
}

/// The environment variable that asks programs for POSIX behaviour, which turns
/// [`ParseFlags::GNU`] off.
const POSIXLY_CORRECT: &str = "POSIXLY_CORRECT";

impl Kind {
    /// Whether a spec of this kind is an option its long name selects, as `--name`: a
    /// [`Bool`](Self::Bool), a [`Switch`](Self::Switch) or a [`Value`](Self::Value).
    ///
    /// The parser selects a spec of any other kind otherwise, or never by its long name: a
    /// bare `--` selects the [`Literal`](Self::Literal), an [`Arg`](Self::Arg) or
    /// [`Args`](Self::Args) takes bare arguments, an [`Accumulator`](Self::Accumulator) is
    /// selected by its short alias alone, as `-v`, and [`None`](Self::None) ends a spec list.
    /// The C parser of long arguments has a branch for an accumulator that this predicate
    /// leaves unreachable, as the C macro does; no spec of the CLI is an accumulator.
    ///
    /// The C macro takes a spec and reads its `type`; here the predicate is the kind's, which
    /// a ported spec's kind answers. It computes on its argument alone, so it is a `const fn`.
    /// A C spec whose `type` holds a value `opt.h` does not name, which the macro reports as
    /// no option, has no `Kind`: its conversion refuses it before the predicate is reached.
    ///
    /// ```
    /// use libgit2_crustify::git2::src::cli::opt::Kind;
    ///
    /// assert!(Kind::Bool.is_option());
    /// assert!(Kind::Value.is_option());
    /// assert!(!Kind::Accumulator.is_option());
    /// assert!(!Kind::Args.is_option());
    /// ```
    ///
    /// Replaces `mac::spec_is_option_type`
    #[doc(alias = "spec_is_option_type")]
    #[must_use]
    #[inline]
    pub const fn is_option(self) -> bool {
        matches!(self, Self::Bool | Self::Switch | Self::Value)
    }
}

/// Whether a parser built with `flags` parses GNU-style, as `getopt_long` does, letting
/// options follow bare arguments (`file1 --help file2`), which it reorders in place: always
/// under [`ParseFlags::FORCE_GNU`]; under [`ParseFlags::GNU`] alone, unless the environment
/// sets `POSIXLY_CORRECT`, to any value, the empty one included; never without either. The
/// flags that ask nothing of the parsing style, those no constant names included, do not
/// change it.
///
/// The C reads the variable with `getenv`, whose result it compares with NULL, which is
/// [`std::env::var_os`] returning `Some`: libc's `getenv` takes its `std` route. The variable
/// is read on every call, so a change to the environment between two parsers changes the
/// second one's style. On Windows the C reads it with `_wgetenv` where `UNICODE` is defined,
/// which `var_os` does too, and the crate is built for Linux alone.
///
/// ```
/// use libgit2_crustify::git2::src::cli::opt::{ParseFlags, uses_gnu_style};
///
/// assert!(uses_gnu_style(ParseFlags::FORCE_GNU));
/// assert!(!uses_gnu_style(ParseFlags::DEFAULT));
/// ```
///
/// Replaces `fn::support_gnu_style`
#[doc(alias = "support_gnu_style")]
#[must_use]
pub fn uses_gnu_style(flags: ParseFlags) -> bool {
    if flags.contains(ParseFlags::FORCE_GNU) {
        return true;
    }
    flags.contains(ParseFlags::GNU) && std::env::var_os(POSIXLY_CORRECT).is_none()
}

/// What a parse does when it meets a spec: the spec's [`Kind`] together with the variable it
/// writes, which C keeps in a spec's `type` and its type-erased `value`.
///
/// Each kind writes one type of variable, which its variant holds; `None` in a variant is a
/// spec with no variable, which C parses without writing anything. The variables are
/// [`Cell`]s, since one variable may serve several specs, as a mode two switches select does:
///
/// - a [`Kind::Bool`] spec sets its `c_int` to `1` when given and to `0` when given negated, as
///   `--no-<name>`;
/// - a [`Kind::Switch`] spec sets it to the spec's switch value;
/// - a [`Kind::Accumulator`] spec adds the spec's switch value to it, or `1` for a switch value of
///   `0`;
/// - a [`Kind::Value`] or [`Kind::Arg`] spec stores the argument it took, as an [`Arg`]; and
/// - a [`Kind::Args`] spec stores where the arguments it took start, as an [`ArgsSlot`].
///
/// [`Kind::None`], the kind of the zeroed spec that ends a table, and [`Kind::Literal`], the
/// bare `--`, write nothing.
#[derive(Clone, Copy, Debug)]
pub enum Target<'p> {
    /// The end of a spec table.
    None,
    /// A flag set to `1`, or to `0` by its `no-` form.
    Bool(Option<&'p Cell<c_int>>),
    /// A variable set to the spec's switch value.
    Switch(Option<&'p Cell<c_int>>),
    /// A counter the spec's switch value is added to each time the option is given.
    Accumulator(Option<&'p Cell<c_int>>),
    /// The value the option was given.
    Value(Option<&'p Cell<Option<Arg<'p>>>>),
    /// The bare `--` after which every argument is literal.
    Literal,
    /// The bare argument this spec matched.
    Arg(Option<&'p Cell<Option<Arg<'p>>>>),
    /// The remaining arguments, from the first one this spec matched.
    Args(Option<&'p ArgsSlot<'p>>),
}

impl Target<'_> {
    /// The kind of spec that writes this target.
    #[must_use]
    pub const fn kind(&self) -> Kind {
        match self {
            Self::None => Kind::None,
            Self::Bool(_) => Kind::Bool,
            Self::Switch(_) => Kind::Switch,
            Self::Accumulator(_) => Kind::Accumulator,
            Self::Value(_) => Kind::Value,
            Self::Literal => Kind::Literal,
            Self::Arg(_) => Kind::Arg,
            Self::Args(_) => Kind::Args,
        }
    }
}

/// A spec's fields set one by one, each taking and returning the spec, so that a spec reads as
/// C's initialiser does, and a table as C's array of them, ended by the zeroed
/// [`Spec::default`]:
///
/// ```
/// use core::cell::Cell;
///
/// use libgit2_crustify::git2::src::cli::opt::{Spec, SpecTable, Target, Usage};
///
/// let debug = Cell::new(0);
/// let specs = [
///     Spec::new(Target::Bool(Some(&debug)))
///         .with_name(c"debug")
///         .with_alias(b'd')
///         .with_help(c"show debugging output"),
///     Spec::new(Target::Literal).with_usage(Usage::HIDDEN),
///     Spec::default(),
/// ];
/// let table = SpecTable::new(&specs).unwrap();
/// assert_eq!(table.get(0).and_then(|s| s.name()), Some(c"debug"));
/// ```
impl<'p> Spec<'p> {
    /// The spec with the long option's name, without its dashes.
    ///
    /// Wraps `fld::cli_opt_spec.name`
    #[doc(alias = "cli_opt_spec.name")]
    #[must_use]
    pub fn with_name(mut self, name: &'p CStr) -> Self {
        self.as_mut().set_name(Some(name));
        self
    }

    /// The spec with the short option's one-character alias, as `b'd'` for `-d`; `0`, as C's
    /// `'\0'`, leaves it without one.
    ///
    /// Wraps `fld::cli_opt_spec.alias`
    #[doc(alias = "cli_opt_spec.alias")]
    #[must_use]
    pub fn with_alias(mut self, alias: u8) -> Self {
        self.as_mut().set_alias(NonZero::new(alias));
        self
    }

    /// The spec with the value a [`Kind::Switch`] spec sets, or a [`Kind::Accumulator`] spec
    /// adds.
    ///
    /// Wraps `fld::cli_opt_spec.switch_value`
    #[doc(alias = "cli_opt_spec.switch_value")]
    #[must_use]
    pub fn with_switch_value(mut self, switch_value: c_int) -> Self {
        self.as_mut().set_switch_value(switch_value);
        self
    }

    /// The spec with how it is parsed and shown in usage.
    ///
    /// Wraps `fld::cli_opt_spec.usage`
    #[doc(alias = "cli_opt_spec.usage")]
    #[must_use]
    pub fn with_usage(mut self, usage: Usage) -> Self {
        self.as_mut().set_usage(usage);
        self
    }

    /// The spec with the name usage shows for its value.
    ///
    /// Wraps `fld::cli_opt_spec.value_name`
    #[doc(alias = "cli_opt_spec.value_name")]
    #[must_use]
    pub fn with_value_name(mut self, value_name: &'p CStr) -> Self {
        self.as_mut().set_value_name(Some(value_name));
        self
    }

    /// The spec with the one-line description usage shows for it.
    ///
    /// Wraps `fld::cli_opt_spec.help`
    #[doc(alias = "cli_opt_spec.help")]
    #[must_use]
    pub fn with_help(mut self, help: &'p CStr) -> Self {
        self.as_mut().set_help(Some(help));
        self
    }
}

/// The variable a [`Kind::Args`] spec writes: where, in the argument array of the parse that
/// matched it, the arguments it took start, as C's `char **`.
///
/// It holds a position, not the arguments: the parser holds its array exclusively for as long
/// as the variables its specs write are in use, and how many arguments the spec took is the
/// outcome's [`args_len`](OptRef::args_len). [`get_in`](Self::get_in) finds them in the array
/// the parser lends, [`ParserRef::args`]. It is never read through, so it cannot dangle.
#[repr(transparent)]
pub struct ArgsSlot<'p>(Cell<*mut Arg<'p>>);

impl<'p> ArgsSlot<'p> {
    /// A slot no parse has written: it holds C's `NULL`.
    #[must_use]
    pub const fn new() -> Self {
        Self(Cell::new(core::ptr::null_mut()))
    }

    /// Whether a parse, or [`set`](Self::set), has stored a position.
    #[must_use]
    pub fn is_set(&self) -> bool {
        !self.0.get().is_null()
    }

    /// Stores the start of `tail`, the arguments from the first one a spec took, as C stores
    /// `&args[idx]`.
    pub fn set(&self, tail: &[Arg<'p>]) {
        self.0.set(tail.as_ptr().cast_mut());
    }

    /// Empties the slot, as [`new`](Self::new) builds it.
    pub fn clear(&self) {
        self.0.set(core::ptr::null_mut());
    }

    /// The arguments of `args` from the stored position on, or `None` when the slot is empty or
    /// its position is not within `args`: its first element through one past its last.
    ///
    /// The position is compared by address alone, so a slot written for another array finds
    /// nothing in this one.
    #[must_use]
    pub fn get_in<'x>(&self, args: &'x [Arg<'p>]) -> Option<&'x [Arg<'p>]> {
        let start = self.0.get();
        if start.is_null() {
            return None;
        }
        let base = args.as_ptr().addr();
        let offset = start.addr().checked_sub(base)?;
        let index = offset / size_of::<Arg<'p>>();
        if offset % size_of::<Arg<'p>>() != 0 {
            return None;
        }
        args.get(index..)
    }
}

impl Default for ArgsSlot<'_> {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for ArgsSlot<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("ArgsSlot").field(&self.0.get()).finish()
    }
}

/// The CLI's three structs as C lays them out, with their handles and accessors, the argument
/// strings they borrow, and the C initialiser of a parser.
#[expect(
    unsafe_code,
    reason = "the C layouts of opt.h's structs, the argument strings they point at, and \
              cli_opt_parser_init through the raw bindings"
)]
mod ffi_import {
    use core::cell::Cell;
    use core::ffi::{CStr, c_char, c_int, c_void};
    use core::fmt;
    use core::marker::PhantomData;
    use core::mem::MaybeUninit;
    use core::num::NonZero;
    use core::ptr::{self, NonNull};

    use ffibox::{CMutSeam, CRefSeam, CSlice, CZeroable, define_ctype};

    use super::{ArgsSlot, Kind, ParseFlags, Status, Target, Usage};
    use crate::sys::git2 as sys;

    /// One command-line argument: a C string borrowed for `'p`, an element of the argument
    /// array a [`Parser`] reads, as C's `argv` holds them.
    ///
    /// `#[repr(transparent)]` over a non-null `char *`, so a `[Arg<'p>]` is the `char *[]` C
    /// passes a parser, and an `Option<Arg<'p>>` a `char *` that may be `NULL`. C only reads
    /// through it, and a parse only moves arguments within their array, never changing their
    /// bytes.
    #[repr(transparent)]
    #[derive(Clone, Copy)]
    pub struct Arg<'p>(NonNull<c_char>, PhantomData<&'p CStr>);

    impl<'p> Arg<'p> {
        /// The argument `arg`, borrowed for `'p`.
        #[must_use]
        pub const fn new(arg: &'p CStr) -> Self {
            Self(NonNull::from_ref(arg).cast(), PhantomData)
        }

        /// The argument's string.
        #[must_use]
        pub fn as_c_str(self) -> &'p CStr {
            // SAFETY: an `Arg<'p>` is built by `new` from a `&'p CStr`, or read from C memory a
            // handle's adoption vouched holds a C string that lives and stays unchanged for
            // `'p`; either way `self.0` addresses a NUL-terminated string nothing writes for
            // `'p`.
            unsafe { CStr::from_ptr(self.0.as_ptr()) }
        }
    }

    impl<'p> From<&'p CStr> for Arg<'p> {
        fn from(arg: &'p CStr) -> Self {
            Self::new(arg)
        }
    }

    impl fmt::Debug for Arg<'_> {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            fmt::Debug::fmt(self.as_c_str(), f)
        }
    }

    /// The string a `const char *` field points at, `None` for `NULL`.
    ///
    /// # Safety
    ///
    /// `p` is `NULL`, or addresses a NUL-terminated string that lives and stays unchanged for
    /// `'p`.
    unsafe fn c_str<'p>(p: *const c_char) -> Option<&'p CStr> {
        // SAFETY: the caller's contract, for a non-null `p`.
        (!p.is_null()).then(|| unsafe { CStr::from_ptr(p) })
    }

    /// The pointer a `char *` field stores for `s`, `NULL` for `None`.
    fn c_str_ptr(s: Option<&CStr>) -> *mut c_char {
        s.map_or(ptr::null_mut(), |s| s.as_ptr().cast_mut())
    }

    define_ctype!(
        /// One option or argument a parser accepts (`cli_opt_spec`), held by value in Rust
        /// storage, as an element of a spec table.
        ///
        /// Built by [`new`](Self::new) from its [`Target`], or [`zeroed`](CZeroable::zeroed)
        /// as the spec that ends a table; read it through [`as_ref`](Self::as_ref) and change it
        /// through [`as_mut`](Self::as_mut).
        ///
        /// `#[repr(transparent)]` over `cli_opt_spec`, so a slice of them is the spec array C
        /// reads in place. `'p` is the lifetime of what it points at: its strings and its
        /// variable are borrowed for `'p`, so the spec cannot outlive them:
        ///
        /// ```compile_fail,E0597
        /// use core::cell::Cell;
        /// use libgit2_crustify::git2::src::cli::opt::{Spec, Target};
        ///
        /// let spec;
        /// {
        ///     let debug = Cell::new(0);
        ///     spec = Spec::new(Target::Bool(Some(&debug)));
        /// }
        /// let _ = spec.as_ref().target();
        /// ```
        ///
        /// It is invariant in `'p`: a spec built for long-lived variables cannot be read as
        /// one for shorter-lived ones, which a parse would then store a shorter-lived argument
        /// into.
        ///
        /// ```compile_fail
        /// use libgit2_crustify::git2::src::cli::opt::SpecRef;
        ///
        /// fn shrink<'a, 'p>(s: SpecRef<'a, 'static>) -> SpecRef<'a, 'p> {
        ///     s
        /// }
        /// ```
        ///
        /// Neither `Send` nor `Sync`: its variables are [`Cell`]s, which a parse on another
        /// thread would race with.
        ///
        /// ```compile_fail,E0277
        /// use libgit2_crustify::git2::src::cli::opt::Spec;
        /// use ffibox::CZeroable;
        ///
        /// let spec: Spec<'static> = Spec::zeroed();
        /// std::thread::spawn(move || drop(spec));
        /// ```
        ///
        /// # Adoption
        ///
        /// A handle adopted from a pointer, through [`CRefSeam::from_ptr`] or
        /// [`CMutSeam::from_ptr`], is read as a `Spec<'p>` for the `'p` its caller names, and
        /// its getters lend what the spec points at for that `'p`, beyond the handle's own
        /// borrow. Those seams' contract, a live, initialised `cli_opt_spec` that outlives the
        /// handle, does not cover it, so their caller also vouches that the spec is a valid
        /// `Spec<'p>`: its `name`, `value_name` and `help` are each `NULL` or a NUL-terminated
        /// string that lives, unchanged, for `'p`, and its `value` is `NULL` or addresses, for
        /// `'p`, a live variable of the type its `type` writes, as [`Target`] lists them, which
        /// only a parse writes.
        ///
        /// Wraps `st::cli_opt_spec`
        ///
        /// Wraps `tdf::cli_opt_spec`
        #[doc(alias = "cli_opt_spec")]
        Spec<'p>, SpecRef, SpecMut, sys::cli_opt_spec, phantom = Cell<&'p CStr>
    );

    // SAFETY: all-zero is a spec of kind `CLI_OPT_TYPE_NONE` with no variable, no alias, no
    // usage flag, a switch value of 0 and `NULL` strings: every getter reads it (`None` for
    // each string, `Target::None`), and C reads it as the spec that ends a table.
    unsafe impl CZeroable for Spec<'_> {}

    impl<'p> Spec<'p> {
        /// A spec of `target`'s kind that writes its variable, with no name, alias, switch
        /// value, usage flag, value name or help, which its setters give it.
        #[must_use]
        pub fn new(target: Target<'p>) -> Self {
            let mut spec = Self::zeroed();
            spec.as_mut().set_target(target);
            spec
        }
    }

    impl Default for Spec<'_> {
        /// The zeroed spec, which ends a table.
        fn default() -> Self {
            Self::zeroed()
        }
    }

    impl fmt::Debug for Spec<'_> {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            self.as_ref().fmt(f)
        }
    }

    impl<'p> SpecRef<'_, 'p> {
        /// The spec's kind, or `None` for a value `opt.h` does not name, which only a table C
        /// built can hold.
        ///
        /// Wraps `fld::cli_opt_spec.type`
        #[doc(alias = "cli_opt_spec.type")]
        #[must_use]
        pub fn kind(&self) -> Option<Kind> {
            Kind::from_raw(self.raw_kind())
        }

        /// The spec's kind as C holds it.
        fn raw_kind(&self) -> u32 {
            let p = self.as_ptr();
            // SAFETY: `p` is the handle's pointer to the live spec it borrows; the projection
            // forms no reference to it.
            let type_ptr = unsafe { &raw const (*p).type_ };
            // SAFETY: a read through the shared handle's pointer to a live spec.
            unsafe { type_ptr.read() }
        }

        /// The spec's kind with the variable a parse writes, or `None` for a kind `opt.h` does
        /// not name.
        ///
        /// Wraps `fld::cli_opt_spec.value`
        ///
        /// Wraps `fld::cli_opt_spec.type`
        #[doc(alias = "cli_opt_spec.value")]
        #[doc(alias = "cli_opt_spec.type")]
        #[must_use]
        pub fn target(&self) -> Option<Target<'p>> {
            let p = self.as_ptr();
            // SAFETY: `p` is the handle's pointer to the live spec it borrows; the projection
            // forms no reference to it.
            let value_ptr = unsafe { &raw const (*p).value };
            // SAFETY: a read through the shared handle's pointer to a live spec.
            let value = unsafe { value_ptr.read() };
            let kind = self.kind()?;
            Some(match kind {
                Kind::None => Target::None,
                Kind::Literal => Target::Literal,
                // SAFETY: `set_target` stores, with each kind, `NULL` or a pointer
                // from a `&'p` borrow of the variable that kind writes, and a spec C built
                // holds, by its handle's adoption, `NULL` or a pointer to such a variable,
                // live for `'p`; C writes it only through that pointer, as the `Cell` allows.
                Kind::Bool => Target::Bool(unsafe { value.cast::<Cell<c_int>>().as_ref() }),
                // SAFETY: as for `Kind::Bool`.
                Kind::Switch => Target::Switch(unsafe { value.cast::<Cell<c_int>>().as_ref() }),
                Kind::Accumulator => {
                    // SAFETY: as for `Kind::Bool`.
                    Target::Accumulator(unsafe { value.cast::<Cell<c_int>>().as_ref() })
                }
                Kind::Value => {
                    // SAFETY: as for `Kind::Bool`, the variable a `char *` C stores a string
                    // in that lives for `'p`.
                    Target::Value(unsafe { value.cast::<Cell<Option<Arg<'p>>>>().as_ref() })
                }
                // SAFETY: as for `Kind::Value`.
                Kind::Arg => Target::Arg(unsafe { value.cast::<Cell<Option<Arg<'p>>>>().as_ref() }),
                // SAFETY: as for `Kind::Bool`, the variable a `char **` C stores a position
                // in, which nothing reads through.
                Kind::Args => Target::Args(unsafe { value.cast::<ArgsSlot<'p>>().as_ref() }),
            })
        }

        /// The long option's name, without its dashes, as `"debug"` for `--debug`.
        ///
        /// Wraps `fld::cli_opt_spec.name`
        #[doc(alias = "cli_opt_spec.name")]
        #[must_use]
        pub fn name(&self) -> Option<&'p CStr> {
            let p = self.as_ptr();
            // SAFETY: `p` is the handle's pointer to the live spec it borrows; the projection
            // forms no reference to it.
            let name_ptr = unsafe { &raw const (*p).name };
            // SAFETY: a read through the shared handle's pointer to a live spec.
            let name = unsafe { name_ptr.read() };
            // SAFETY: `set_name` stores `NULL` or a `&'p CStr`'s pointer, and a spec C built
            // holds, by its handle's adoption, `NULL` or a string that lives unchanged for `'p`.
            unsafe { c_str(name) }
        }

        /// The short option's one-character alias, as `b'd'` for `-d`, or `None` for a spec
        /// without one.
        ///
        /// Wraps `fld::cli_opt_spec.alias`
        #[doc(alias = "cli_opt_spec.alias")]
        #[must_use]
        pub fn alias(&self) -> Option<NonZero<u8>> {
            let p = self.as_ptr();
            // SAFETY: `p` is the handle's pointer to the live spec it borrows; the projection
            // forms no reference to it.
            let alias_ptr = unsafe { &raw const (*p).alias };
            // SAFETY: a read through the shared handle's pointer to a live spec.
            let alias = unsafe { alias_ptr.read() };
            NonZero::new(u8::from_ne_bytes(alias.to_ne_bytes()))
        }

        /// The value a [`Kind::Switch`] spec sets, or a [`Kind::Accumulator`] spec adds, `1`
        /// standing in for `0` there; other kinds ignore it.
        ///
        /// Wraps `fld::cli_opt_spec.switch_value`
        #[doc(alias = "cli_opt_spec.switch_value")]
        #[must_use]
        pub fn switch_value(&self) -> c_int {
            let p = self.as_ptr();
            // SAFETY: `p` is the handle's pointer to the live spec it borrows; the projection
            // forms no reference to it.
            let switch_value_ptr = unsafe { &raw const (*p).switch_value };
            // SAFETY: a read through the shared handle's pointer to a live spec.
            unsafe { switch_value_ptr.read() }
        }

        /// How the spec is parsed and shown in usage.
        ///
        /// Wraps `fld::cli_opt_spec.usage`
        #[doc(alias = "cli_opt_spec.usage")]
        #[must_use]
        pub fn usage(&self) -> Usage {
            let p = self.as_ptr();
            // SAFETY: `p` is the handle's pointer to the live spec it borrows; the projection
            // forms no reference to it.
            let usage_ptr = unsafe { &raw const (*p).usage };
            // SAFETY: a read through the shared handle's pointer to a live spec.
            Usage::from_bits_retain(unsafe { usage_ptr.read() })
        }

        /// The name usage shows for the spec's value, as `"key=value"`.
        ///
        /// Wraps `fld::cli_opt_spec.value_name`
        #[doc(alias = "cli_opt_spec.value_name")]
        #[must_use]
        pub fn value_name(&self) -> Option<&'p CStr> {
            let p = self.as_ptr();
            // SAFETY: `p` is the handle's pointer to the live spec it borrows; the projection
            // forms no reference to it.
            let value_name_ptr = unsafe { &raw const (*p).value_name };
            // SAFETY: a read through the shared handle's pointer to a live spec.
            let value_name = unsafe { value_name_ptr.read() };
            // SAFETY: as for `name`, through `set_value_name`.
            unsafe { c_str(value_name) }
        }

        /// The one-line description usage shows for the spec.
        ///
        /// Wraps `fld::cli_opt_spec.help`
        #[doc(alias = "cli_opt_spec.help")]
        #[must_use]
        pub fn help(&self) -> Option<&'p CStr> {
            let p = self.as_ptr();
            // SAFETY: `p` is the handle's pointer to the live spec it borrows; the projection
            // forms no reference to it.
            let help_ptr = unsafe { &raw const (*p).help };
            // SAFETY: a read through the shared handle's pointer to a live spec.
            let help = unsafe { help_ptr.read() };
            // SAFETY: as for `name`, through `set_help`.
            unsafe { c_str(help) }
        }
    }

    impl fmt::Debug for SpecRef<'_, '_> {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.debug_struct("Spec")
                .field("target", &self.target())
                .field("name", &self.name())
                .field("alias", &self.alias())
                .field("switch_value", &self.switch_value())
                .field("usage", &self.usage())
                .field("value_name", &self.value_name())
                .field("help", &self.help())
                .finish()
        }
    }

    impl<'p> SpecMut<'_, 'p> {
        /// Sets the spec's kind and the variable a parse writes, both at once, so that the
        /// variable is always of the type its kind writes.
        ///
        /// Wraps `fld::cli_opt_spec.value`
        ///
        /// Wraps `fld::cli_opt_spec.type`
        #[doc(alias = "cli_opt_spec.value")]
        #[doc(alias = "cli_opt_spec.type")]
        pub fn set_target(&mut self, target: Target<'p>) {
            fn erase<T>(variable: Option<&T>) -> *mut c_void {
                variable.map_or(ptr::null_mut(), |v| ptr::from_ref(v).cast_mut().cast())
            }
            let value = match target {
                Target::None | Target::Literal => ptr::null_mut(),
                Target::Bool(v) | Target::Switch(v) | Target::Accumulator(v) => erase(v),
                Target::Value(v) | Target::Arg(v) => erase(v),
                Target::Args(v) => erase(v),
            };
            let p = self.as_mut_ptr();
            // SAFETY: `p` is the exclusive handle's pointer to the live spec it borrows; the
            // projection forms no reference to it.
            let type_ptr = unsafe { &raw mut (*p).type_ };
            // SAFETY: as above.
            let value_ptr = unsafe { &raw mut (*p).value };
            // SAFETY: a write through the exclusive handle's pointer to a live spec; the field
            // is plain data, so the old value needs no drop.
            unsafe { type_ptr.write(target.kind().raw()) };
            // SAFETY: as above. The pointer is `NULL` or from a `&'p` borrow of the variable
            // the kind just written writes, as `target()` requires; it is derived from a shared
            // borrow of a `Cell` or an `ArgsSlot`, whose contents may be written through it.
            unsafe { value_ptr.write(value) };
        }

        /// Sets the long option's name, without its dashes.
        ///
        /// Wraps `fld::cli_opt_spec.name`
        #[doc(alias = "cli_opt_spec.name")]
        pub fn set_name(&mut self, name: Option<&'p CStr>) {
            let p = self.as_mut_ptr();
            // SAFETY: `p` is the exclusive handle's pointer to the live spec it borrows; the
            // projection forms no reference to it.
            let name_ptr = unsafe { &raw mut (*p).name };
            // SAFETY: a write through the exclusive handle's pointer to a live spec; the field
            // is plain data. The string is borrowed for `'p`, as `name()` requires.
            unsafe { name_ptr.write(c_str_ptr(name)) };
        }

        /// Sets the short option's one-character alias, or removes it.
        ///
        /// Wraps `fld::cli_opt_spec.alias`
        #[doc(alias = "cli_opt_spec.alias")]
        pub fn set_alias(&mut self, alias: Option<NonZero<u8>>) {
            let alias = c_char::from_ne_bytes(alias.map_or(0, NonZero::get).to_ne_bytes());
            let p = self.as_mut_ptr();
            // SAFETY: `p` is the exclusive handle's pointer to the live spec it borrows; the
            // projection forms no reference to it.
            let alias_ptr = unsafe { &raw mut (*p).alias };
            // SAFETY: a write through the exclusive handle's pointer to a live spec; the field
            // is plain data.
            unsafe { alias_ptr.write(alias) };
        }

        /// Sets the value a [`Kind::Switch`] spec sets, or a [`Kind::Accumulator`] spec adds.
        ///
        /// Wraps `fld::cli_opt_spec.switch_value`
        #[doc(alias = "cli_opt_spec.switch_value")]
        pub fn set_switch_value(&mut self, switch_value: c_int) {
            let p = self.as_mut_ptr();
            // SAFETY: `p` is the exclusive handle's pointer to the live spec it borrows; the
            // projection forms no reference to it.
            let switch_value_ptr = unsafe { &raw mut (*p).switch_value };
            // SAFETY: a write through the exclusive handle's pointer to a live spec; the field
            // is plain data.
            unsafe { switch_value_ptr.write(switch_value) };
        }

        /// Sets how the spec is parsed and shown in usage.
        ///
        /// Wraps `fld::cli_opt_spec.usage`
        #[doc(alias = "cli_opt_spec.usage")]
        pub fn set_usage(&mut self, usage: Usage) {
            let p = self.as_mut_ptr();
            // SAFETY: `p` is the exclusive handle's pointer to the live spec it borrows; the
            // projection forms no reference to it.
            let usage_ptr = unsafe { &raw mut (*p).usage };
            // SAFETY: a write through the exclusive handle's pointer to a live spec; the field
            // is plain data.
            unsafe { usage_ptr.write(usage.bits()) };
        }

        /// Sets the name usage shows for the spec's value.
        ///
        /// Wraps `fld::cli_opt_spec.value_name`
        #[doc(alias = "cli_opt_spec.value_name")]
        pub fn set_value_name(&mut self, value_name: Option<&'p CStr>) {
            let p = self.as_mut_ptr();
            // SAFETY: `p` is the exclusive handle's pointer to the live spec it borrows; the
            // projection forms no reference to it.
            let value_name_ptr = unsafe { &raw mut (*p).value_name };
            // SAFETY: a write through the exclusive handle's pointer to a live spec; the field
            // is plain data. The string is borrowed for `'p`, as `value_name()` requires.
            unsafe { value_name_ptr.write(c_str_ptr(value_name)) };
        }

        /// Sets the one-line description usage shows for the spec.
        ///
        /// Wraps `fld::cli_opt_spec.help`
        #[doc(alias = "cli_opt_spec.help")]
        pub fn set_help(&mut self, help: Option<&'p CStr>) {
            let p = self.as_mut_ptr();
            // SAFETY: `p` is the exclusive handle's pointer to the live spec it borrows; the
            // projection forms no reference to it.
            let help_ptr = unsafe { &raw mut (*p).help };
            // SAFETY: a write through the exclusive handle's pointer to a live spec; the field
            // is plain data. The string is borrowed for `'p`, as `help()` requires.
            unsafe { help_ptr.write(c_str_ptr(help)) };
        }
    }

    impl fmt::Debug for SpecMut<'_, '_> {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            self.as_ref().fmt(f)
        }
    }

    /// A spec table: the specs a parser accepts, in order, borrowed for `'p`, as C passes them,
    /// a `const cli_opt_spec[]` whose end is a spec of kind [`Kind::None`].
    ///
    /// Its length counts the specs before that end, which C never matches; the end itself
    /// stays in memory after them, for C to stop at.
    #[derive(Clone, Copy)]
    pub struct SpecTable<'p> {
        /// The specs before the end, which follows them in the same allocation.
        specs: CSlice<'p, Spec<'p>>,
    }

    impl<'p> SpecTable<'p> {
        /// The table of `specs`, up to its first spec of kind [`Kind::None`], or `None` when
        /// `specs` holds no such spec to end it.
        #[must_use]
        pub fn new(specs: &'p [Spec<'p>]) -> Option<Self> {
            let len = specs.iter().position(|s| s.as_ref().raw_kind() == 0)?;
            // SAFETY: `specs` is borrowed shared for `'p`, so its first `len` elements, ended by
            // the spec at `len`, live and stay unchanged for `'p`.
            let specs = unsafe { CSlice::from_raw_parts(NonNull::from(specs).cast(), len) };
            Some(Self { specs })
        }

        /// The table that starts at `p`, ended by its first spec of kind `CLI_OPT_TYPE_NONE`.
        ///
        /// # Safety
        ///
        /// `p` is non-null and addresses an array of specs, ended by one of kind
        /// `CLI_OPT_TYPE_NONE`, that lives and stays unchanged for `'p`, whose specs' strings
        /// and variables live for `'p`, each variable of the type its spec's kind writes.
        unsafe fn from_c(p: NonNull<sys::cli_opt_spec>) -> Self {
            let base = p.as_ptr();
            let mut len = 0;
            loop {
                let spec = base.wrapping_add(len);
                // SAFETY: the caller's contract: the array holds every spec up to its end, and
                // `spec` is one of them while no end was seen; the projection forms no
                // reference to it.
                let type_ptr = unsafe { &raw const (*spec).type_ };
                // SAFETY: a read of that live spec.
                if unsafe { type_ptr.read() } == 0 {
                    break;
                }
                len += 1;
            }
            // SAFETY: the caller's contract; `Spec` is `#[repr(transparent)]` over
            // `cli_opt_spec`.
            let specs = unsafe { CSlice::from_raw_parts(p.cast(), len) };
            Self { specs }
        }

        /// The specs' start, which C reads up to the end.
        fn as_c_ptr(&self) -> *const sys::cli_opt_spec {
            self.specs.as_ptr()
        }

        /// How many specs precede the end.
        #[must_use]
        pub const fn len(&self) -> usize {
            self.specs.len()
        }

        /// Whether the table is only its end.
        #[must_use]
        pub const fn is_empty(&self) -> bool {
            self.specs.is_empty()
        }

        /// The spec at `index`, or `None` from the end on.
        #[must_use]
        pub fn get(&self, index: usize) -> Option<SpecRef<'p, 'p>> {
            self.specs.get(index)
        }

        /// The specs, in order, without the end.
        pub fn iter(&self) -> impl Iterator<Item = SpecRef<'p, 'p>> + use<'p> {
            self.specs.iter()
        }

        /// Where `spec` sits in the table, or `None` when it is not one of its specs, compared by
        /// address as C compares spec pointers.
        #[must_use]
        pub fn position(&self, spec: SpecRef<'_, 'p>) -> Option<usize> {
            let offset = spec.as_ptr().addr().checked_sub(self.as_c_ptr().addr())?;
            let index = offset / size_of::<Spec<'p>>();
            (offset % size_of::<Spec<'p>>() == 0 && index < self.len()).then_some(index)
        }
    }

    impl fmt::Debug for SpecTable<'_> {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.debug_list().entries(self.iter()).finish()
        }
    }

    define_ctype!(
        /// The outcome of parsing one command-line argument (`cli_opt`), held by value in Rust
        /// storage.
        ///
        /// [`zeroed`](CZeroable::zeroed), as C clears one before each argument: status
        /// [`Status::Done`], no spec, argument or value, and no arguments taken. Read it through
        /// [`as_ref`](Self::as_ref) and change it through [`as_mut`](Self::as_mut).
        ///
        /// `#[repr(transparent)]` over `cli_opt`. `'p` is the lifetime of what it points at: the
        /// spec it matched, from its parser's table, and the argument and value, from its
        /// parser's arguments, so it cannot outlive them:
        ///
        /// ```compile_fail,E0597
        /// use libgit2_crustify::git2::src::cli::opt::Opt;
        /// use ffibox::CZeroable;
        /// use std::ffi::CString;
        ///
        /// let mut opt = Opt::zeroed();
        /// {
        ///     let arg = CString::new("--debug").unwrap();
        ///     opt.as_mut().set_arg(Some(&arg));
        /// }
        /// let _ = opt.as_ref().arg();
        /// ```
        ///
        /// Neither `Send` nor `Sync`, as its spec is neither.
        ///
        /// ```compile_fail,E0277
        /// use libgit2_crustify::git2::src::cli::opt::Opt;
        /// use ffibox::CZeroable;
        ///
        /// let opt: Opt<'static> = Opt::zeroed();
        /// std::thread::spawn(move || drop(opt));
        /// ```
        ///
        /// # Adoption
        ///
        /// As for a [`Spec`]: the caller of [`CRefSeam::from_ptr`] or [`CMutSeam::from_ptr`]
        /// also vouches that the outcome is a valid `Opt<'p>` for the `'p` it names: its `spec`
        /// is `NULL` or addresses a valid `Spec<'p>`, as `Spec`'s adoption asks, that lives,
        /// unchanged, for `'p`, and its `arg` and `value` are each `NULL` or a NUL-terminated
        /// string that lives, unchanged, for `'p`.
        ///
        /// Wraps `st::cli_opt`
        ///
        /// Wraps `tdf::cli_opt`
        #[doc(alias = "cli_opt")]
        Opt<'p>, OptRef, OptMut, sys::cli_opt, phantom = Cell<&'p CStr>
    );

    // SAFETY: all-zero is the outcome `cli_opt_parser_next` clears before each argument:
    // status `CLI_OPT_STATUS_DONE`, `NULL` spec, argument and value, and 0 arguments taken,
    // which every getter reads.
    unsafe impl CZeroable for Opt<'_> {}

    impl Default for Opt<'_> {
        /// The zeroed outcome.
        fn default() -> Self {
            Self::zeroed()
        }
    }

    impl fmt::Debug for Opt<'_> {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            self.as_ref().fmt(f)
        }
    }

    impl<'p> OptRef<'_, 'p> {
        /// The outcome of the argument, or `None` for a value `opt.h` does not name, which only
        /// an outcome C wrote can hold.
        ///
        /// Wraps `fld::cli_opt.status`
        #[doc(alias = "cli_opt.status")]
        #[must_use]
        pub fn status(&self) -> Option<Status> {
            let p = self.as_ptr();
            // SAFETY: `p` is the handle's pointer to the live outcome it borrows; the
            // projection forms no reference to it.
            let status_ptr = unsafe { &raw const (*p).status };
            // SAFETY: a read through the shared handle's pointer to a live outcome.
            Status::from_raw(unsafe { status_ptr.read() })
        }

        /// The spec the argument matched, from its parser's table, or `None` for an argument
        /// that matched none.
        ///
        /// Wraps `fld::cli_opt.spec`
        #[doc(alias = "cli_opt.spec")]
        #[must_use]
        pub fn spec(&self) -> Option<SpecRef<'p, 'p>> {
            let p = self.as_ptr();
            // SAFETY: `p` is the handle's pointer to the live outcome it borrows; the
            // projection forms no reference to it.
            let spec_ptr = unsafe { &raw const (*p).spec };
            // SAFETY: a read through the shared handle's pointer to a live outcome.
            let spec = unsafe { spec_ptr.read() };
            // SAFETY: `set_spec` stores `NULL` or the pointer of a `SpecRef<'p, 'p>`, a spec
            // live and unchanged for `'p`, and an outcome C wrote holds, by its handle's
            // adoption, `NULL` or a spec of its parser's table, which lives for `'p`.
            unsafe { SpecRef::from_ptr(spec.cast_mut()) }
        }

        /// The argument as it was given, dashes included, as `"-f"` or `"--foo"`.
        ///
        /// Wraps `fld::cli_opt.arg`
        #[doc(alias = "cli_opt.arg")]
        #[must_use]
        pub fn arg(&self) -> Option<&'p CStr> {
            let p = self.as_ptr();
            // SAFETY: `p` is the handle's pointer to the live outcome it borrows; the
            // projection forms no reference to it.
            let arg_ptr = unsafe { &raw const (*p).arg };
            // SAFETY: a read through the shared handle's pointer to a live outcome.
            let arg = unsafe { arg_ptr.read() };
            // SAFETY: `set_arg` stores `NULL` or a `&'p CStr`'s pointer, and an outcome C wrote
            // holds, by its handle's adoption, `NULL` or one of its parser's arguments, which
            // live unchanged for `'p`.
            unsafe { c_str(arg) }
        }

        /// The value a [`Kind::Value`] spec was given: the rest of its argument, as `"bar"` of
        /// `--foo=bar` or `-fbar`, or the argument after it.
        ///
        /// Wraps `fld::cli_opt.value`
        #[doc(alias = "cli_opt.value")]
        #[must_use]
        pub fn value(&self) -> Option<&'p CStr> {
            let p = self.as_ptr();
            // SAFETY: `p` is the handle's pointer to the live outcome it borrows; the
            // projection forms no reference to it.
            let value_ptr = unsafe { &raw const (*p).value };
            // SAFETY: a read through the shared handle's pointer to a live outcome.
            let value = unsafe { value_ptr.read() };
            // SAFETY: as for `arg`, through `set_value`; C stores an argument or a suffix of one.
            unsafe { c_str(value) }
        }

        /// How many arguments a [`Kind::Args`] spec took, which the parser keeps reporting once
        /// it is done.
        ///
        /// Wraps `fld::cli_opt.args_len`
        #[doc(alias = "cli_opt.args_len")]
        #[must_use]
        pub fn args_len(&self) -> usize {
            let p = self.as_ptr();
            // SAFETY: `p` is the handle's pointer to the live outcome it borrows; the
            // projection forms no reference to it.
            let args_len_ptr = unsafe { &raw const (*p).args_len };
            // SAFETY: a read through the shared handle's pointer to a live outcome.
            unsafe { args_len_ptr.read() }
        }
    }

    impl fmt::Debug for OptRef<'_, '_> {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.debug_struct("Opt")
                .field("status", &self.status())
                .field("spec", &self.spec())
                .field("arg", &self.arg())
                .field("value", &self.value())
                .field("args_len", &self.args_len())
                .finish()
        }
    }

    impl<'p> OptMut<'_, 'p> {
        /// Sets the outcome of the argument.
        ///
        /// Wraps `fld::cli_opt.status`
        #[doc(alias = "cli_opt.status")]
        pub fn set_status(&mut self, status: Status) {
            let p = self.as_mut_ptr();
            // SAFETY: `p` is the exclusive handle's pointer to the live outcome it borrows; the
            // projection forms no reference to it.
            let status_ptr = unsafe { &raw mut (*p).status };
            // SAFETY: a write through the exclusive handle's pointer to a live outcome; the
            // field is plain data.
            unsafe { status_ptr.write(status.raw()) };
        }

        /// Sets the spec the argument matched, or that it matched none.
        ///
        /// Wraps `fld::cli_opt.spec`
        #[doc(alias = "cli_opt.spec")]
        pub fn set_spec(&mut self, spec: Option<SpecRef<'p, 'p>>) {
            let spec = spec.map_or(ptr::null(), |s| s.as_ptr());
            let p = self.as_mut_ptr();
            // SAFETY: `p` is the exclusive handle's pointer to the live outcome it borrows; the
            // projection forms no reference to it.
            let spec_ptr = unsafe { &raw mut (*p).spec };
            // SAFETY: a write through the exclusive handle's pointer to a live outcome; the
            // field is plain data. The spec is borrowed for `'p`, as `spec()` requires.
            unsafe { spec_ptr.write(spec.cast()) };
        }

        /// Sets the argument as it was given.
        ///
        /// Wraps `fld::cli_opt.arg`
        #[doc(alias = "cli_opt.arg")]
        pub fn set_arg(&mut self, arg: Option<&'p CStr>) {
            let p = self.as_mut_ptr();
            // SAFETY: `p` is the exclusive handle's pointer to the live outcome it borrows; the
            // projection forms no reference to it.
            let arg_ptr = unsafe { &raw mut (*p).arg };
            // SAFETY: a write through the exclusive handle's pointer to a live outcome; the
            // field is plain data. The string is borrowed for `'p`, as `arg()` requires, and
            // nothing writes through the `char *`.
            unsafe { arg_ptr.write(c_str_ptr(arg)) };
        }

        /// Sets the value the argument's spec was given.
        ///
        /// Wraps `fld::cli_opt.value`
        #[doc(alias = "cli_opt.value")]
        pub fn set_value(&mut self, value: Option<&'p CStr>) {
            let p = self.as_mut_ptr();
            // SAFETY: `p` is the exclusive handle's pointer to the live outcome it borrows; the
            // projection forms no reference to it.
            let value_ptr = unsafe { &raw mut (*p).value };
            // SAFETY: as for `set_arg`, of `value()`.
            unsafe { value_ptr.write(c_str_ptr(value)) };
        }

        /// Sets how many arguments a [`Kind::Args`] spec took.
        ///
        /// Wraps `fld::cli_opt.args_len`
        #[doc(alias = "cli_opt.args_len")]
        pub fn set_args_len(&mut self, args_len: usize) {
            let p = self.as_mut_ptr();
            // SAFETY: `p` is the exclusive handle's pointer to the live outcome it borrows; the
            // projection forms no reference to it.
            let args_len_ptr = unsafe { &raw mut (*p).args_len };
            // SAFETY: a write through the exclusive handle's pointer to a live outcome; the
            // field is plain data.
            unsafe { args_len_ptr.write(args_len) };
        }
    }

    impl fmt::Debug for OptMut<'_, '_> {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            self.as_ref().fmt(f)
        }
    }

    define_ctype!(
        /// A parser's state (`cli_opt_parser`) over a spec table and an argument array, held by
        /// value in Rust storage.
        ///
        /// Built by [`new`](Self::new), as C's `cli_opt_parser_init` sets one up, or
        /// [`zeroed`](CZeroable::zeroed), a parser with no table and no arguments. Read it
        /// through [`as_ref`](Self::as_ref) and change it through [`as_mut`](Self::as_mut).
        ///
        /// `#[repr(transparent)]` over `cli_opt_parser`, which C reads and writes in place.
        /// `'p` is the lifetime of what it points at: the spec table, borrowed shared, and the
        /// argument array, borrowed exclusively, since a GNU-style parse reorders it. So the
        /// arguments cannot be reached while the parser may still be used:
        ///
        /// ```compile_fail,E0506
        /// use libgit2_crustify::git2::src::cli::opt::{Arg, ParseFlags, Parser, Spec, SpecTable};
        /// use ffibox::CZeroable;
        ///
        /// let specs = [Spec::zeroed()];
        /// let table = SpecTable::new(&specs).unwrap();
        /// let mut args = [Arg::new(c"file")];
        /// let parser = Parser::new(table, &mut args, ParseFlags::DEFAULT);
        /// args[0] = Arg::new(c"other");
        /// let _ = parser.as_ref().idx();
        /// ```
        ///
        /// Not `Clone`, since it holds that exclusive borrow, and neither `Send` nor `Sync`, as
        /// its specs are neither.
        ///
        /// ```compile_fail,E0277
        /// use libgit2_crustify::git2::src::cli::opt::Parser;
        /// use ffibox::CZeroable;
        ///
        /// let parser: Parser<'static> = Parser::zeroed();
        /// std::thread::spawn(move || drop(parser));
        /// ```
        ///
        /// # Adoption
        ///
        /// As for a [`Spec`]: the caller of [`CRefSeam::from_ptr`] or [`CMutSeam::from_ptr`]
        /// also vouches that the parser is a valid `Parser<'p>` for the `'p` it names: its
        /// `specs` is `NULL` or addresses a run of valid `Spec<'p>`s, as `Spec`'s adoption asks,
        /// ended by one of kind `CLI_OPT_TYPE_NONE`, that lives, unchanged, for `'p`; and its
        /// `args` is `NULL`, its `args_len` is `0`, or it addresses `args_len` non-null
        /// arguments, each a NUL-terminated string that lives, unchanged, for `'p`, in an
        /// array that nothing but the parser's handles, and C given the parser, reaches for
        /// `'p`.
        ///
        /// Wraps `st::cli_opt_parser`
        ///
        /// Wraps `tdf::cli_opt_parser`
        #[doc(alias = "cli_opt_parser")]
        Parser<'p>, ParserRef, ParserMut, sys::cli_opt_parser, phantom = &'p mut [Arg<'p>]
    );

    // SAFETY: all-zero is a parser with no spec table (`NULL`), no arguments (`NULL`, 0 of
    // them), no flag and every position and bit 0, which every getter reads (`None`, an empty
    // slice), and which C's `cli_opt_parser_next` reports done at once, reading neither.
    unsafe impl CZeroable for Parser<'_> {}

    impl<'p> Parser<'p> {
        /// A parser of `args` against `specs` under `flags`, at its first argument: it will
        /// reorder `args` GNU style when `flags` asks for it, [`ParseFlags::FORCE_GNU`], or
        /// [`ParseFlags::GNU`] while the environment has no `POSIXLY_CORRECT`, read here.
        ///
        /// Wraps `fn::cli_opt_parser_init`
        #[doc(alias = "cli_opt_parser_init")]
        #[must_use]
        pub fn new(specs: SpecTable<'p>, args: &'p mut [Arg<'p>], flags: ParseFlags) -> Self {
            let mut raw = MaybeUninit::<sys::cli_opt_parser>::uninit();
            // SAFETY: `raw` is writable storage for one parser, which `cli_opt_parser_init`
            // zeroes whole before it stores the table, the array and its length, and the flags;
            // it keeps no pointer to `raw`, reads no argument and no spec, and only reads the
            // environment, which safe Rust cannot change meanwhile. The table and the array
            // are borrowed for `'p`, as the getters require, the array exclusively, through
            // the pointer stored.
            unsafe {
                sys::cli_opt_parser_init(
                    raw.as_mut_ptr(),
                    specs.as_c_ptr(),
                    args.as_mut_ptr().cast(),
                    args.len(),
                    flags.bits(),
                );
            }
            // SAFETY: `cli_opt_parser_init` initialised every byte: `memset` first, then each
            // field it sets.
            Self(unsafe { raw.assume_init() }, PhantomData)
        }
    }

    impl Default for Parser<'_> {
        /// The zeroed parser.
        fn default() -> Self {
            Self::zeroed()
        }
    }

    impl fmt::Debug for Parser<'_> {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            self.as_ref().fmt(f)
        }
    }

    impl<'a, 'p> ParserRef<'a, 'p> {
        /// The spec table the parser matches arguments against, or `None` for a parser that has
        /// none, as a zeroed one.
        ///
        /// Wraps `fld::cli_opt_parser.specs`
        #[doc(alias = "cli_opt_parser.specs")]
        #[must_use]
        pub fn specs(&self) -> Option<SpecTable<'p>> {
            let p = self.as_ptr();
            // SAFETY: `p` is the handle's pointer to the live parser it borrows; the projection
            // forms no reference to it.
            let specs_ptr = unsafe { &raw const (*p).specs };
            // SAFETY: a read through the shared handle's pointer to a live parser.
            let specs = NonNull::new(unsafe { specs_ptr.read() }.cast_mut())?;
            // SAFETY: `new` and `set_specs` store a `SpecTable<'p>`'s start, an array ended by
            // a spec of kind `CLI_OPT_TYPE_NONE` borrowed shared for `'p`, and a parser C set up
            // holds, by its handle's adoption, such a table, with its strings and variables.
            Some(unsafe { SpecTable::from_c(specs) })
        }

        /// The arguments the parser reads, in their current order, which a GNU-style parse
        /// changes.
        ///
        /// Wraps `fld::cli_opt_parser.args`
        #[doc(alias = "cli_opt_parser.args")]
        #[must_use]
        pub fn args(&self) -> &'a [Arg<'p>] {
            let p = self.as_ptr();
            // SAFETY: `p` is the handle's pointer to the live parser it borrows; the
            // projection forms no reference to it.
            let args_ptr = unsafe { &raw const (*p).args };
            // SAFETY: as above.
            let len_ptr = unsafe { &raw const (*p).args_len };
            // SAFETY: a read through the shared handle's pointer to a live parser.
            let args = unsafe { args_ptr.read() };
            // SAFETY: as above.
            let len = unsafe { len_ptr.read() };
            if args.is_null() || len == 0 {
                return &[];
            }
            // SAFETY: `new` and `set_args` store an `&'p mut [Arg<'p>]`'s start and length, and
            // a parser C set up holds, by its handle's adoption, an array of `len` non-null
            // arguments borrowed for `'p`; `Arg` is `#[repr(transparent)]` over a non-null
            // `char *`. Only the exclusive handle, or C given the parser, writes the array, and
            // neither can while this shared borrow of the parser lives.
            unsafe { core::slice::from_raw_parts(args.cast::<Arg<'p>>(), len) }
        }

        /// How many arguments the parser reads.
        ///
        /// Wraps `fld::cli_opt_parser.args_len`
        #[doc(alias = "cli_opt_parser.args_len")]
        #[must_use]
        pub fn args_len(&self) -> usize {
            let p = self.as_ptr();
            // SAFETY: `p` is the handle's pointer to the live parser it borrows; the projection
            // forms no reference to it.
            let args_len_ptr = unsafe { &raw const (*p).args_len };
            // SAFETY: a read through the shared handle's pointer to a live parser.
            unsafe { args_len_ptr.read() }
        }

        /// The flags the parser was set up with.
        ///
        /// Wraps `fld::cli_opt_parser.flags`
        #[doc(alias = "cli_opt_parser.flags")]
        #[must_use]
        pub fn flags(&self) -> ParseFlags {
            let p = self.as_ptr();
            // SAFETY: `p` is the handle's pointer to the live parser it borrows; the projection
            // forms no reference to it.
            let flags_ptr = unsafe { &raw const (*p).flags };
            // SAFETY: a read through the shared handle's pointer to a live parser.
            ParseFlags::from_bits_retain(unsafe { flags_ptr.read() })
        }

        /// The index of the next argument to parse.
        ///
        /// Wraps `fld::cli_opt_parser.idx`
        #[doc(alias = "cli_opt_parser.idx")]
        #[must_use]
        pub fn idx(&self) -> usize {
            let p = self.as_ptr();
            // SAFETY: `p` is the handle's pointer to the live parser it borrows; the projection
            // forms no reference to it.
            let idx_ptr = unsafe { &raw const (*p).idx };
            // SAFETY: a read through the shared handle's pointer to a live parser.
            unsafe { idx_ptr.read() }
        }

        /// How many of the table's [`Kind::Arg`] specs bare arguments have matched, or a choice
        /// has made unnecessary.
        ///
        /// Wraps `fld::cli_opt_parser.arg_idx`
        #[doc(alias = "cli_opt_parser.arg_idx")]
        #[must_use]
        pub fn arg_idx(&self) -> usize {
            let p = self.as_ptr();
            // SAFETY: `p` is the handle's pointer to the live parser it borrows; the projection
            // forms no reference to it.
            let arg_idx_ptr = unsafe { &raw const (*p).arg_idx };
            // SAFETY: a read through the shared handle's pointer to a live parser.
            unsafe { arg_idx_ptr.read() }
        }

        /// How many arguments a [`Kind::Args`] spec took, `0` until one has.
        ///
        /// Wraps `fld::cli_opt_parser.in_args`
        #[doc(alias = "cli_opt_parser.in_args")]
        #[must_use]
        pub fn in_args(&self) -> usize {
            let p = self.as_ptr();
            // SAFETY: `p` is the handle's pointer to the live parser it borrows; the projection
            // forms no reference to it.
            let in_args_ptr = unsafe { &raw const (*p).in_args };
            // SAFETY: a read through the shared handle's pointer to a live parser.
            unsafe { in_args_ptr.read() }
        }

        /// How many short options of the current argument's bundle, as `-fbcd`, were parsed
        /// before the next one, `0` outside a bundle.
        ///
        /// Wraps `fld::cli_opt_parser.in_short`
        #[doc(alias = "cli_opt_parser.in_short")]
        #[must_use]
        pub fn in_short(&self) -> usize {
            let p = self.as_ptr();
            // SAFETY: `p` is the handle's pointer to the live parser it borrows; the projection
            // forms no reference to it.
            let in_short_ptr = unsafe { &raw const (*p).in_short };
            // SAFETY: a read through the shared handle's pointer to a live parser.
            unsafe { in_short_ptr.read() }
        }

        /// Whether the parser still has to move the options after the first bare argument
        /// before it, GNU style, which it does once.
        ///
        /// Wraps `fld::cli_opt_parser.needs_sort`
        #[doc(alias = "cli_opt_parser.needs_sort")]
        #[must_use]
        pub fn needs_sort(&self) -> bool {
            // SAFETY: the handle's pointer to the live parser it borrows; the accessor reads
            // the bit-field's storage through a projection, forming no reference to it.
            unsafe { sys::cli_opt_parser::needs_sort_raw(self.as_ptr()) != 0 }
        }

        /// Whether a bare `--` was parsed, after which every argument is literal.
        ///
        /// Wraps `fld::cli_opt_parser.in_literal`
        #[doc(alias = "cli_opt_parser.in_literal")]
        #[must_use]
        pub fn in_literal(&self) -> bool {
            // SAFETY: as for `needs_sort`.
            unsafe { sys::cli_opt_parser::in_literal_raw(self.as_ptr()) != 0 }
        }
    }

    impl fmt::Debug for ParserRef<'_, '_> {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.debug_struct("Parser")
                .field("specs", &self.specs())
                .field("args", &self.args())
                .field("flags", &self.flags())
                .field("idx", &self.idx())
                .field("arg_idx", &self.arg_idx())
                .field("in_args", &self.in_args())
                .field("in_short", &self.in_short())
                .field("needs_sort", &self.needs_sort())
                .field("in_literal", &self.in_literal())
                .finish()
        }
    }

    impl<'p> ParserMut<'_, 'p> {
        /// Sets the spec table the parser matches arguments against, or leaves it none.
        ///
        /// Wraps `fld::cli_opt_parser.specs`
        #[doc(alias = "cli_opt_parser.specs")]
        pub fn set_specs(&mut self, specs: Option<SpecTable<'p>>) {
            let specs = specs.map_or(ptr::null(), |s| s.as_c_ptr());
            let p = self.as_mut_ptr();
            // SAFETY: `p` is the exclusive handle's pointer to the live parser it borrows; the
            // projection forms no reference to it.
            let specs_ptr = unsafe { &raw mut (*p).specs };
            // SAFETY: a write through the exclusive handle's pointer to a live parser; the
            // field is plain data. The table is borrowed for `'p`, as `specs()` requires.
            unsafe { specs_ptr.write(specs) };
        }

        /// Sets the arguments the parser reads, and their count, borrowing them exclusively for
        /// `'p`; the position is left as it is.
        ///
        /// Wraps `fld::cli_opt_parser.args`
        ///
        /// Wraps `fld::cli_opt_parser.args_len`
        #[doc(alias = "cli_opt_parser.args")]
        #[doc(alias = "cli_opt_parser.args_len")]
        pub fn set_args(&mut self, args: &'p mut [Arg<'p>]) {
            let len = args.len();
            let args = args.as_mut_ptr().cast::<*mut c_char>();
            let p = self.as_mut_ptr();
            // SAFETY: `p` is the exclusive handle's pointer to the live parser it borrows; the
            // projection forms no reference to it.
            let args_ptr = unsafe { &raw mut (*p).args };
            // SAFETY: as above.
            let len_ptr = unsafe { &raw mut (*p).args_len };
            // SAFETY: a write through the exclusive handle's pointer to a live parser; the
            // field is plain data. The array is borrowed exclusively for `'p`, as `args()`
            // and `args_mut()` require, with its length written beside it.
            unsafe { args_ptr.write(args) };
            // SAFETY: as above.
            unsafe { len_ptr.write(len) };
        }

        /// The arguments the parser reads, to reorder, as a GNU-style parse moves its options
        /// ahead of the bare arguments.
        ///
        /// Wraps `fld::cli_opt_parser.args`
        #[doc(alias = "cli_opt_parser.args")]
        pub fn args_mut(&mut self) -> &mut [Arg<'p>] {
            let p = self.as_mut_ptr();
            // SAFETY: `p` is the exclusive handle's pointer to the live parser it borrows; the
            // projection forms no reference to it.
            let args_ptr = unsafe { &raw const (*p).args };
            // SAFETY: as above.
            let len_ptr = unsafe { &raw const (*p).args_len };
            // SAFETY: a read through the exclusive handle's pointer to a live parser.
            let args = unsafe { args_ptr.read() };
            // SAFETY: as above.
            let len = unsafe { len_ptr.read() };
            if args.is_null() || len == 0 {
                return &mut [];
            }
            // SAFETY: as for `ParserRef::args`, and the array is borrowed exclusively for `'p`
            // through the parser, whose exclusive handle this borrows for the result's
            // lifetime, so no other borrow of the array lives meanwhile. Any `Arg<'p>` written
            // is a valid element.
            unsafe { core::slice::from_raw_parts_mut(args.cast::<Arg<'p>>(), len) }
        }

        /// Sets the flags the parser reads; [`Parser::new`] decided from them whether it
        /// reorders.
        ///
        /// Wraps `fld::cli_opt_parser.flags`
        #[doc(alias = "cli_opt_parser.flags")]
        pub fn set_flags(&mut self, flags: ParseFlags) {
            let p = self.as_mut_ptr();
            // SAFETY: `p` is the exclusive handle's pointer to the live parser it borrows; the
            // projection forms no reference to it.
            let flags_ptr = unsafe { &raw mut (*p).flags };
            // SAFETY: a write through the exclusive handle's pointer to a live parser; the
            // field is plain data.
            unsafe { flags_ptr.write(flags.bits()) };
        }

        /// Sets the index of the next argument to parse.
        ///
        /// Wraps `fld::cli_opt_parser.idx`
        #[doc(alias = "cli_opt_parser.idx")]
        pub fn set_idx(&mut self, idx: usize) {
            let p = self.as_mut_ptr();
            // SAFETY: `p` is the exclusive handle's pointer to the live parser it borrows; the
            // projection forms no reference to it.
            let idx_ptr = unsafe { &raw mut (*p).idx };
            // SAFETY: a write through the exclusive handle's pointer to a live parser; the
            // field is plain data, which no accessor indexes with.
            unsafe { idx_ptr.write(idx) };
        }

        /// Sets how many of the table's [`Kind::Arg`] specs are done with.
        ///
        /// Wraps `fld::cli_opt_parser.arg_idx`
        #[doc(alias = "cli_opt_parser.arg_idx")]
        pub fn set_arg_idx(&mut self, arg_idx: usize) {
            let p = self.as_mut_ptr();
            // SAFETY: `p` is the exclusive handle's pointer to the live parser it borrows; the
            // projection forms no reference to it.
            let arg_idx_ptr = unsafe { &raw mut (*p).arg_idx };
            // SAFETY: a write through the exclusive handle's pointer to a live parser; the
            // field is plain data.
            unsafe { arg_idx_ptr.write(arg_idx) };
        }

        /// Sets how many arguments a [`Kind::Args`] spec took.
        ///
        /// Wraps `fld::cli_opt_parser.in_args`
        #[doc(alias = "cli_opt_parser.in_args")]
        pub fn set_in_args(&mut self, in_args: usize) {
            let p = self.as_mut_ptr();
            // SAFETY: `p` is the exclusive handle's pointer to the live parser it borrows; the
            // projection forms no reference to it.
            let in_args_ptr = unsafe { &raw mut (*p).in_args };
            // SAFETY: a write through the exclusive handle's pointer to a live parser; the
            // field is plain data.
            unsafe { in_args_ptr.write(in_args) };
        }

        /// Sets how many short options of the current argument's bundle were parsed.
        ///
        /// Wraps `fld::cli_opt_parser.in_short`
        #[doc(alias = "cli_opt_parser.in_short")]
        pub fn set_in_short(&mut self, in_short: usize) {
            let p = self.as_mut_ptr();
            // SAFETY: `p` is the exclusive handle's pointer to the live parser it borrows; the
            // projection forms no reference to it.
            let in_short_ptr = unsafe { &raw mut (*p).in_short };
            // SAFETY: a write through the exclusive handle's pointer to a live parser; the
            // field is plain data, which no accessor indexes with.
            unsafe { in_short_ptr.write(in_short) };
        }

        /// Sets whether the parser still has to reorder its arguments GNU style.
        ///
        /// Wraps `fld::cli_opt_parser.needs_sort`
        #[doc(alias = "cli_opt_parser.needs_sort")]
        pub fn set_needs_sort(&mut self, needs_sort: bool) {
            // SAFETY: the exclusive handle's pointer to the live parser it borrows; the
            // accessor rewrites the bit-field's storage through a projection, forming no
            // reference to it, and leaves the other bit as it was.
            unsafe {
                sys::cli_opt_parser::set_needs_sort_raw(self.as_mut_ptr(), needs_sort.into());
            };
        }

        /// Sets whether every argument from here on is literal.
        ///
        /// Wraps `fld::cli_opt_parser.in_literal`
        #[doc(alias = "cli_opt_parser.in_literal")]
        pub fn set_in_literal(&mut self, in_literal: bool) {
            // SAFETY: as for `set_needs_sort`.
            unsafe {
                sys::cli_opt_parser::set_in_literal_raw(self.as_mut_ptr(), in_literal.into());
            };
        }
    }

    impl fmt::Debug for ParserMut<'_, '_> {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            self.as_ref().fmt(f)
        }
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
