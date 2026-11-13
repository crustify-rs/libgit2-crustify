//! `src/cli/opt.c`: the command-line argument parser of the `git2` CLI (`src/cli/opt.h`),
//! ported to Rust.
//!
//! The header's enums are native Rust: [`Kind`] and [`Status`] are closed enums, whose
//! discriminants stay C's values while the CLI's remaining C, its `cmd_*.c` spec tables,
//! `opt_usage.c` and `common.c`, uses their constants, and [`Usage`] and [`ParseFlags`] are
//! bit sets, which that C combines with `|`. `opt.h` keeps the C definitions for that C:
//! the two are not linked.
//!
//! The parser's helpers are native Rust too: [`Kind::is_option`], `opt.c`'s
//! `spec_is_option_type`, and [`uses_gnu_style`], its `support_gnu_style`, and the routines
//! through which the parser finds the spec an argument selects, steps over a choice and checks
//! the required arguments. No C outside `opt.c` reaches them, so none is exported.
//!
//! # Its entry points
//!
//! [`Parser::new`] (`cli_opt_parser_init`) sets a parser up and [`ParserMut::parse_next`]
//! (`cli_opt_parser_next`) parses its arguments one by one, reordering them GNU style where it
//! was asked to; [`parse`] (`cli_opt_parse`) parses a whole command line and checks its required
//! arguments, and [`for_each`] (`cli_opt_foreach`) calls a [`ForeachCallbackFn`] with each
//! outcome, which a Rust closure or, through the export, `common.c`'s C function implements.
//! [`OptRef::write_status`] (`cli_opt_status_fprint`) describes an outcome, as the CLI shows it on
//! its standard error. The CLI's remaining C calls all five: `main.c` sets a parser up, steps it
//! and describes its failures, each `cmd_*.c` parses its own arguments, `common.c` reads the
//! configuration options, and `opt_usage.c` describes failures. So each has an export, compiled
//! under the module's export feature, `ffi-export-src-cli-opt-c`, which stays off until the
//! module switches from C.
//!
//! The parser corrects the C's defects, each
//! documented on the routine that departs: it reads no string past its end, nor through a null
//! pointer, and keeps any number of the specs a parse gives, where the C writes past a stack
//! array of one per argument; an unknown option ends its bundle; and GNU-style reordering moves
//! nothing after a literal `--`, and loses no bare argument before an option whose value is
//! optional.
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
use core::num::NonZero;
use core::ops::{BitAnd, BitAndAssign, BitOr, BitOrAssign, ControlFlow};
use core::{fmt, ptr};
use std::io;

pub use ffi_import::{
    Arg, ForeachCallbackFn, Opt, OptMut, OptRef, Parser, ParserMut, ParserRef, Spec, SpecMut,
    SpecRef, SpecTable,
};
use ffibox::{CRefSeam, CZeroable};

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
        Self(Cell::new(ptr::null_mut()))
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
        self.0.set(ptr::null_mut());
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

impl<'p> Parser<'p> {
    /// A parser of `args` against `specs` under `flags`, at its first argument: it will
    /// reorder `args` GNU style when `flags` asks for it, [`ParseFlags::FORCE_GNU`], or
    /// [`ParseFlags::GNU`] while the environment has no `POSIXLY_CORRECT`, read here, as
    /// [`uses_gnu_style`] reads it.
    ///
    /// Every other piece of its state starts cleared, as C's `memset` leaves it: no argument
    /// parsed, no [`Kind::Arg`] spec matched, no remaining arguments taken, no bundle of short
    /// options entered and no literal `--` seen. It reads neither the table nor the arguments.
    ///
    /// ```
    /// use libgit2_crustify::git2::src::cli::opt::{Arg, ParseFlags, Parser, Spec, SpecTable};
    ///
    /// let specs = [Spec::default()];
    /// let table = SpecTable::new(&specs).unwrap();
    /// let mut args = [Arg::new(c"file")];
    /// let parser = Parser::new(table, &mut args, ParseFlags::FORCE_GNU);
    /// assert_eq!((parser.as_ref().args_len(), parser.as_ref().idx()), (1, 0));
    /// assert!(parser.as_ref().needs_sort());
    /// ```
    ///
    /// Replaces `fn::cli_opt_parser_init`
    #[doc(alias = "cli_opt_parser_init")]
    #[must_use]
    pub fn new(specs: SpecTable<'p>, args: &'p mut [Arg<'p>], flags: ParseFlags) -> Self {
        let mut parser = Self::zeroed();
        {
            let mut state = parser.as_mut();
            state.set_specs(Some(specs));
            state.set_args(args);
            state.set_flags(flags);
            state.set_needs_sort(uses_gnu_style(flags));
        }
        parser
    }
}

/// How a long argument, `--<arg>`, selects the spec [`find_long`] finds for it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LongForm<'p> {
    /// By the spec's name alone, as `--name`, or the bare `--` of a [`Kind::Literal`] spec.
    Plain,
    /// By the `no-` form of a [`Kind::Bool`] spec's name, as `--no-name`, which sets it to 0.
    Negated,
    /// By a [`Kind::Value`] spec's name with its value after an `=`, as `--name=value`: the
    /// value, or `None` for the empty one of `--name=`.
    Valued(Option<&'p CStr>),
}

/// The spec a long argument selects, with how it selects it: `arg` is the argument after its
/// `--`, as `debug` of `--debug`, the empty string for the bare `--`. `None` when the parser
/// has no table, or no spec of its table matches.
///
/// The table's specs are tried in order, and the first that matches is the one: within each,
/// a [`Kind::Literal`] spec matches the bare `--`, a [`Kind::Bool`] spec its name after `no-`,
/// a spec whose kind [`is_option`](Kind::is_option) its name, and a [`Kind::Value`] spec its
/// name before the first `=` of `arg`. Names compare as bytes, as `strcmp` compares them.
///
/// The C compares `arg` after its `no-` with a bool spec's name without checking that the spec
/// has one, which reads through a null pointer: a bool spec without a name matches no `no-`
/// argument here, as it matches no other.
///
/// Replaces `fn::spec_for_long`
#[doc(alias = "spec_for_long")]
fn find_long<'p>(
    parser: ParserRef<'_, 'p>,
    arg: &'p CStr,
) -> Option<(SpecRef<'p, 'p>, LongForm<'p>)> {
    let bytes = arg.to_bytes();
    let eql = bytes.iter().position(|&b| b == b'=');
    parser.specs()?.iter().find_map(|spec| {
        let kind = spec.kind();
        let name = spec.name().map(CStr::to_bytes);
        if kind == Some(Kind::Literal) && bytes.is_empty() {
            return Some((spec, LongForm::Plain));
        }
        if kind == Some(Kind::Bool) && name.is_some() && bytes.strip_prefix(b"no-") == name {
            return Some((spec, LongForm::Negated));
        }
        if kind.is_some_and(Kind::is_option) && name == Some(bytes) {
            return Some((spec, LongForm::Plain));
        }
        if kind == Some(Kind::Value)
            && let (Some(name), Some(at)) = (name, eql)
            && bytes.get(..at) == Some(name)
        {
            let value = arg
                .to_bytes_with_nul()
                .get(at + 1..)
                .and_then(|rest| CStr::from_bytes_until_nul(rest).ok())
                .filter(|value| !value.is_empty());
            return Some((spec, LongForm::Valued(value)));
        }
        None
    })
}

/// The spec a short option selects, with the value it carries: `arg` is the rest of the
/// argument from the option's character on, as `ofile` of `-ofile`, or `vd` of the bundle `-xvd`
/// once `x` is parsed. `None` when the parser has no table, or no spec of its table has that
/// character as its alias.
///
/// The first spec whose alias is the character is the one. A [`Kind::Value`] spec takes the rest
/// of `arg` after it as its value, as `file` of `-ofile`, when there is any; every other spec,
/// and a value spec at the end of `arg`, carries none.
///
/// The C compares the character with every spec's alias, `0` for a spec without one, so that
/// an empty `arg`, the bare `-`, selects the first spec without an alias, and a value spec
/// without one then has the byte after the string's end read: a spec without an alias matches
/// no short option here, and the empty `arg` matches none.
///
/// Replaces `fn::spec_for_short`
#[doc(alias = "spec_for_short")]
fn find_short<'p>(
    parser: ParserRef<'_, 'p>,
    arg: &'p CStr,
) -> Option<(SpecRef<'p, 'p>, Option<&'p CStr>)> {
    let first = *arg.to_bytes().first()?;
    let spec = parser
        .specs()?
        .iter()
        .find(|spec| spec.alias().map(NonZero::get) == Some(first))?;
    let value = if spec.kind() == Some(Kind::Value) {
        arg.to_bytes_with_nul()
            .get(1..)
            .and_then(|rest| CStr::from_bytes_until_nul(rest).ok())
            .filter(|value| !value.is_empty())
    } else {
        None
    };
    Some((spec, value))
}

/// The spec the parser's next bare argument selects: the [`Kind::Arg`] spec of the table that
/// as many bare arguments as the parser's [`arg_idx`](ParserRef::arg_idx) precede, which it
/// then counts as matched, or, when a [`Kind::Args`] spec comes first, that spec, which takes
/// every argument left and counts nothing. `None` when the parser has no table, or every
/// argument spec is matched.
///
/// Replaces `fn::spec_for_arg`
#[doc(alias = "spec_for_arg")]
fn find_arg<'p>(parser: &mut ParserMut<'_, 'p>) -> Option<SpecRef<'p, 'p>> {
    let specs = parser.as_ref().specs()?;
    let arg_idx = parser.as_ref().arg_idx();
    let mut args = 0;
    for spec in specs.iter() {
        match spec.kind() {
            Some(Kind::Arg) if args == arg_idx => {
                parser.set_arg_idx(arg_idx.wrapping_add(1));
                return Some(spec);
            }
            Some(Kind::Arg) => args += 1,
            Some(Kind::Args) if args == arg_idx => return Some(spec),
            _ => {}
        }
    }
    None
}

/// Whether the spec at `index` of `specs` starts or continues a choice: whether the spec after
/// it is one of the choices it offers, a spec of [`Usage::CHOICE`]. The last spec of the table,
/// and an index past it, are no choice.
///
/// Replaces `fn::spec_is_choice`
#[doc(alias = "spec_is_choice")]
fn is_choice(specs: SpecTable<'_>, index: usize) -> bool {
    index
        .checked_add(1)
        .and_then(|next| specs.get(next))
        .is_some_and(|next| next.usage().contains(Usage::CHOICE))
}

/// Whether the spec at `index` of `specs` is a later choice of a choice before it.
fn is_later_choice(specs: SpecTable<'_>, index: usize) -> bool {
    specs
        .get(index)
        .is_some_and(|spec| spec.usage().contains(Usage::CHOICE))
}

/// Counts as matched the [`Kind::Arg`] specs of the choice `spec` belongs to, once one of its
/// options is given, since a choice of switches and bare arguments takes one of them: the
/// parser's [`arg_idx`](ParserRef::arg_idx) moves past each argument spec of the choice. Nothing
/// changes when `spec` is not in the parser's table, or belongs to no choice.
///
/// The choice starts at the spec before `spec`'s run of [`Usage::CHOICE`] specs. The C steps
/// back over that run without a bound, before the table's start when its first spec is marked
/// [`Usage::CHOICE`]: the table's start ends the run here, as the start of its choice.
///
/// Replaces `fn::consume_choices`
#[doc(alias = "consume_choices")]
fn skip_choice_args<'p>(spec: SpecRef<'_, 'p>, parser: &mut ParserMut<'_, 'p>) {
    let Some(specs) = parser.as_ref().specs() else {
        return;
    };
    let Some(mut start) = specs.position(spec) else {
        return;
    };
    while start > 0 && is_later_choice(specs, start) {
        start -= 1;
    }
    if !is_choice(specs, start) {
        return;
    }
    let mut consumed = 0_usize;
    let mut index = start;
    loop {
        if specs.get(index).and_then(|s| s.kind()) == Some(Kind::Arg) {
            consumed += 1;
        }
        index += 1;
        if !is_later_choice(specs, index) {
            break;
        }
    }
    let arg_idx = parser.as_ref().arg_idx();
    parser.set_arg_idx(arg_idx.wrapping_add(consumed));
}

/// Whether `spec` is one of `given`, the specs the arguments matched, compared by address, as C
/// compares spec pointers.
///
/// Replaces `fn::spec_included`
#[doc(alias = "spec_included")]
fn is_given(given: &[SpecRef<'_, '_>], spec: SpecRef<'_, '_>) -> bool {
    given.iter().any(|g| ptr::eq(g.as_ptr(), spec.as_ptr()))
}

/// Checks that the arguments gave every required spec of `specs`, `given` being the specs they
/// matched: a spec of [`Usage::REQUIRED`], or, when it starts a choice, one of its choices. The
/// first requirement not given is the error: its spec, the first of its choice.
///
/// Replaces `fn::validate_required`
#[doc(alias = "validate_required")]
fn check_required<'p>(
    specs: SpecTable<'p>,
    given: &[SpecRef<'_, 'p>],
) -> Result<(), SpecRef<'p, 'p>> {
    let mut required = None;
    let mut met = false;
    for (index, spec) in specs.iter().enumerate() {
        let head = match required {
            Some(head) => head,
            None if spec.usage().contains(Usage::REQUIRED) => {
                met = false;
                spec
            }
            None => continue,
        };
        required = Some(head);
        met = met || is_given(given, spec);
        if !is_choice(specs, index) {
            if !met {
                return Err(head);
            }
            required = None;
            met = false;
        }
    }
    Ok(())
}

/// The string of `arg` from its byte `at` on, the empty one at its end, or `None` past it: the
/// `&arg[at]` C forms, which it reads past the string's end when `at` is beyond it.
fn suffix(arg: &CStr, at: usize) -> Option<&CStr> {
    arg.to_bytes_with_nul()
        .get(at..)
        .and_then(|rest| CStr::from_bytes_until_nul(rest).ok())
}

/// Whether an outcome that matched `spec` with `value` is [`Status::MissingValue`]: `spec` is a
/// [`Kind::Value`] spec given no value, which [`Usage::VALUE_OPTIONAL`] does not make optional.
fn lacks_value(spec: SpecRef<'_, '_>, value: Option<&CStr>) -> bool {
    spec.kind() == Some(Kind::Value)
        && value.is_none()
        && !spec.usage().contains(Usage::VALUE_OPTIONAL)
}

/// Adds the step an [`Kind::Accumulator`] spec counts by, its switch value or `1` for `0`, to
/// `count`. C's `+=` on an `int` that overflows is undefined; here it wraps.
fn accumulate(count: &Cell<c_int>, spec: SpecRef<'_, '_>) {
    let step = match spec.switch_value() {
        0 => 1,
        step => step,
    };
    count.set(count.get().wrapping_add(step));
}

/// The argument after the one being parsed, which a [`Kind::Value`] spec given no value of its
/// own takes, as `bar` of `--foo bar` or `-f bar`, whatever it looks like: the parser moves past
/// it. `None`, the parser unmoved, when no argument is left.
fn take_following<'p>(parser: &mut ParserMut<'_, 'p>) -> Option<&'p CStr> {
    let idx = parser.as_ref().idx();
    let value = parser.as_ref().args().get(idx).copied()?;
    parser.set_idx(idx + 1);
    Some(value.as_c_str())
}

/// Parses the long argument at the parser's position, `--<name>`, which it moves past: the outcome
/// of the spec [`find_long`] finds for `<name>`, or [`Status::UnknownOption`] when it finds none.
///
/// The spec found writes its variable, if it has one: a [`Kind::Bool`] spec `1`, or `0` for its
/// `no-` form; a [`Kind::Switch`] spec its switch value; a [`Kind::Value`] spec the value after
/// its `=`, or else the next argument, which the parser then moves past too. A [`Kind::Literal`]
/// spec, the bare `--`, makes every later argument literal. A value spec given no value is
/// [`Status::MissingValue`], unless its value is optional, and an outcome of a choice counts
/// its argument specs as matched, as [`skip_choice_args`] does.
///
/// Replaces `fn::parse_long`
#[doc(alias = "parse_long")]
fn take_long<'p>(parser: &mut ParserMut<'_, 'p>) -> Opt<'p> {
    let mut opt = Opt::zeroed();
    let idx = parser.as_ref().idx();
    let Some(arg) = parser.as_ref().args().get(idx).copied() else {
        return opt;
    };
    let arg = arg.as_c_str();
    parser.set_idx(idx + 1);
    let mut out = opt.as_mut();
    out.set_arg(Some(arg));
    let found = suffix(arg, 2).and_then(|name| find_long(parser.as_ref(), name));
    let Some((spec, form)) = found else {
        out.set_status(Status::UnknownOption);
        return opt;
    };
    out.set_spec(Some(spec));
    let mut value = None;
    match spec.target() {
        Some(Target::Literal) => parser.set_in_literal(true),
        Some(Target::Bool(Some(flag))) => flag.set(c_int::from(form != LongForm::Negated)),
        Some(Target::Accumulator(Some(count))) => accumulate(count, spec),
        Some(Target::Switch(Some(mode))) => mode.set(spec.switch_value()),
        Some(Target::Value(variable)) => {
            value = match form {
                LongForm::Valued(given) => given,
                LongForm::Plain | LongForm::Negated => take_following(parser),
            };
            if let Some(variable) = variable {
                variable.set(value.map(Arg::new));
            }
        }
        _ => {}
    }
    out.set_value(value);
    out.set_status(if lacks_value(spec, value) {
        Status::MissingValue
    } else {
        Status::Ok
    });
    skip_choice_args(spec, parser);
    opt
}

/// Parses the short option at the parser's position: `-<c>`, or the next option of a bundle of
/// them, as the `b` of `-abc` once `a` is parsed. The outcome of the spec [`find_short`] finds for
/// the option's character, or [`Status::UnknownOption`] when it finds none.
///
/// The spec found writes its variable, if it has one, as [`take_long`]'s do, a [`Kind::Bool`]
/// spec always `1`: a [`Kind::Value`] spec takes the rest of the argument as its value, as `foo` of
/// `-ofoo`, or else the next argument. An option of any other kind followed by more of the bundle
/// leaves the parser on the argument, at the bundle's next option; otherwise the parser moves past
/// it, and past a value it took. The outcome's argument is the whole bundle, as C gives it.
///
/// An unknown option ends its bundle: the parser moves to the next argument, at its start. The C
/// moves to the next argument but stays as far into it as it was into the bundle, so that it
/// parses that argument from the wrong character, past its end when the argument is shorter.
///
/// Replaces `fn::parse_short`
#[doc(alias = "parse_short")]
fn take_short<'p>(parser: &mut ParserMut<'_, 'p>) -> Opt<'p> {
    let mut opt = Opt::zeroed();
    let (idx, in_short) = (parser.as_ref().idx(), parser.as_ref().in_short());
    let Some(arg) = parser.as_ref().args().get(idx).copied() else {
        return opt;
    };
    let arg = arg.as_c_str();
    parser.set_idx(idx + 1);
    let mut out = opt.as_mut();
    out.set_arg(Some(arg));
    let rest = in_short.checked_add(1).and_then(|at| suffix(arg, at));
    let found = rest.and_then(|rest| Some((rest, find_short(parser.as_ref(), rest)?)));
    let Some((rest, (spec, attached))) = found else {
        parser.set_in_short(0);
        out.set_status(Status::UnknownOption);
        return opt;
    };
    out.set_spec(Some(spec));
    let mut value = None;
    match spec.target() {
        Some(Target::Bool(Some(flag))) => flag.set(1),
        Some(Target::Accumulator(Some(count))) => accumulate(count, spec),
        Some(Target::Switch(Some(mode))) => mode.set(spec.switch_value()),
        Some(Target::Value(variable)) => {
            value = attached.or_else(|| take_following(parser));
            if let Some(variable) = variable {
                variable.set(value.map(Arg::new));
            }
        }
        _ => {}
    }
    if spec.kind() != Some(Kind::Value) && rest.to_bytes().len() > 1 {
        parser.set_in_short(in_short + 1);
        parser.set_idx(idx);
    } else {
        parser.set_in_short(0);
    }
    out.set_value(value);
    out.set_status(if lacks_value(spec, value) {
        Status::MissingValue
    } else {
        Status::Ok
    });
    skip_choice_args(spec, parser);
    opt
}

/// Parses the bare argument at the parser's position, which it moves past: the outcome of the
/// spec [`find_arg`] gives it, or [`Status::UnknownOption`] when every argument spec is taken.
///
/// A [`Kind::Arg`] spec stores the argument in its variable; a [`Kind::Args`] spec stores where
/// the remaining arguments start, this one included, in its [`ArgsSlot`], counts them as the
/// outcome's and the parser's [`args_len`](OptRef::args_len), and moves the parser past them all.
///
/// Replaces `fn::parse_arg`
#[doc(alias = "parse_arg")]
fn take_bare<'p>(parser: &mut ParserMut<'_, 'p>) -> Opt<'p> {
    let mut opt = Opt::zeroed();
    let spec = find_arg(parser);
    let (idx, len) = (parser.as_ref().idx(), parser.as_ref().args_len());
    let arg = parser.as_ref().args().get(idx).copied();
    let mut out = opt.as_mut();
    out.set_spec(spec);
    out.set_arg(arg.map(Arg::as_c_str));
    let Some(spec) = spec else {
        parser.set_idx(idx + 1);
        out.set_status(Status::UnknownOption);
        return opt;
    };
    match spec.target() {
        Some(Target::Args(slot)) => {
            if let Some(slot) = slot {
                slot.set(parser.as_ref().args().get(idx..).unwrap_or_default());
            }
            let taken = len.saturating_sub(idx);
            parser.set_in_args(taken);
            parser.set_idx(len);
            out.set_args_len(taken);
        }
        target => {
            if let Some(Target::Arg(Some(variable))) = target {
                variable.set(arg);
            }
            parser.set_idx(idx + 1);
        }
    }
    out.set_status(Status::Ok);
    opt
}

/// The spec `arg` selects when the parser reorders its arguments GNU style, with whether a
/// [`Kind::Value`] spec selected still needs a value from the next argument: `None` for an
/// argument that is no option, or no option the parser knows.
///
/// A long argument, `--<name>`, selects what [`find_long`] finds, which needs a value unless
/// it gave one after an `=`. A short one, `-<c>…`, is read as a bundle, to its last option or to
/// the first that takes the rest as its value, as `-xvffile`'s `f`: that option is the one,
/// which needs a value only when the bundle gave none. A bundle holding an unknown option, or
/// the bare `-`, selects nothing, where the C selects the first spec without an alias for the
/// bare `-` and reads past its end, as [`find_short`] describes.
///
/// Replaces `fn::spec_for_sort`
#[doc(alias = "spec_for_sort")]
fn find_for_sort<'p>(parser: ParserRef<'_, 'p>, arg: &'p CStr) -> Option<(SpecRef<'p, 'p>, bool)> {
    let bytes = arg.to_bytes();
    if bytes.starts_with(b"--") {
        let (spec, form) = find_long(parser, suffix(arg, 2)?)?;
        return Some((spec, !matches!(form, LongForm::Valued(_))));
    }
    if !bytes.starts_with(b"-") {
        return None;
    }
    let mut at = 1;
    loop {
        let (spec, value) = find_short(parser, suffix(arg, at)?)?;
        at += 1;
        if value.is_some() || at >= bytes.len() {
            return Some((spec, value.is_none()));
        }
    }
}

/// Moves the options after the parser's position ahead of the bare arguments there, keeping
/// their order and those of the bare arguments, each option with the value it takes from the
/// next argument, so that `file1 --help file2` parses as `--help file1 file2`, as GNU
/// `getopt_long` does: whether it moved any. The parser does it once: it no longer needs to.
///
/// An option is what [`find_for_sort`] selects. A bare `--` the parser knows, a
/// [`Kind::Literal`] spec, ends the reordering: what follows it is literal. A [`Kind::Value`]
/// spec that needs a value with no argument after it stops the reordering with the parser moved
/// to it, so that the parse reports its missing value next, unless its value is optional
/// ([`Usage::VALUE_OPTIONAL`]): the option is the last argument, and stays where it is. An
/// option at the parser's first argument, which no parse gives this routine, moves nothing.
///
/// Two departures from the C, which reorders whatever its parser's state: a parser that has
/// parsed a literal `--` reorders nothing, since every argument after it is literal, where the
/// C moves the arguments that look like its options ahead of the others; and an option whose
/// value is optional ends no reordering, where the C moves the parser to it past the bare
/// arguments before it, which no outcome then reports.
///
/// Replaces `fn::sort_gnu_style`
#[doc(alias = "sort_gnu_style")]
fn reorder_gnu_style(parser: &mut ParserMut<'_, '_>) -> bool {
    parser.set_needs_sort(false);
    if parser.as_ref().in_literal() {
        return false;
    }
    let len = parser.as_ref().args_len();
    let mut insert = parser.as_ref().idx();
    let mut at = insert;
    let mut changed = false;
    while let Some(arg) = parser.as_ref().args().get(at).copied() {
        let Some((spec, needs_value)) = find_for_sort(parser.as_ref(), arg.as_c_str()) else {
            at += 1;
            continue;
        };
        if spec.kind() == Some(Kind::Literal) {
            break;
        }
        let mut offset = 0;
        if spec.kind() == Some(Kind::Value) && needs_value {
            if at + 1 < len {
                offset = 1;
            } else if spec.usage().contains(Usage::VALUE_OPTIONAL) {
                break;
            } else {
                parser.set_idx(at);
                return true;
            }
        }
        if at == 0 {
            return false;
        }
        if let Some(run) = parser.args_mut().get_mut(insert..=at + offset) {
            run.rotate_right(1 + offset);
        }
        insert += 1 + offset;
        at += 1 + offset;
        changed = true;
    }
    changed
}

impl<'p> ParserMut<'_, 'p> {
    /// Parses the argument at the parser's position, or the next option of the bundle it is in,
    /// and moves past it: its outcome, with the spec it matched, the argument as given and the
    /// value it took, and [`Status::Done`] once no argument is left, which reports how many
    /// arguments a [`Kind::Args`] spec took, if one did. The specs' variables are written as the
    /// argument selects them.
    ///
    /// An argument that starts with `--` is a long option, `--name`, `--name=value` or
    /// `--no-name`, and one that starts with `-` a short option or a bundle of them, `-abc` or
    /// `-ovalue`: each is looked up among the table's options, and one the table does not have is
    /// [`Status::UnknownOption`]. A [`Kind::Value`] option given no value takes the next
    /// argument, or is [`Status::MissingValue`] when none is left and its value is not optional.
    /// Any other argument is bare, as is every argument after a literal `--`: it takes the next of
    /// the table's [`Kind::Arg`] specs, in order, or a [`Kind::Args`] spec, which takes every
    /// argument left; once they are taken, a bare argument is [`Status::UnknownOption`]. An
    /// option of a choice ([`Usage::CHOICE`]) given counts the choice's argument specs as taken.
    ///
    /// A parser that parses GNU style, as [`Parser::new`] decides, reorders its arguments at the
    /// first bare one it meets, moving the options after it, with their values, ahead of it,
    /// so that options may follow bare arguments, as `file1 --help file2`; it parses in the
    /// order given otherwise, every argument after the first bare one being bare.
    ///
    /// ```
    /// use core::cell::Cell;
    ///
    /// use libgit2_crustify::git2::src::cli::opt::{
    ///     Arg, ParseFlags, Parser, Spec, SpecTable, Status, Target,
    /// };
    ///
    /// let (debug, file) = (Cell::new(0), Cell::new(None));
    /// let specs = [
    ///     Spec::new(Target::Bool(Some(&debug))).with_name(c"debug"),
    ///     Spec::new(Target::Arg(Some(&file))),
    ///     Spec::default(),
    /// ];
    /// let table = SpecTable::new(&specs).unwrap();
    /// let mut args = [c"notes.txt", c"--debug"].map(Arg::new);
    /// let mut parser = Parser::new(table, &mut args, ParseFlags::FORCE_GNU);
    /// let mut parser = parser.as_mut();
    /// let first = parser.parse_next();
    /// assert_eq!(first.as_ref().status(), Some(Status::Ok));
    /// assert_eq!(first.as_ref().arg(), Some(c"--debug"));
    /// assert_eq!(parser.parse_next().as_ref().arg(), Some(c"notes.txt"));
    /// assert_eq!(parser.parse_next().as_ref().status(), Some(Status::Done));
    /// assert_eq!(debug.get(), 1);
    /// assert_eq!(file.get().map(Arg::as_c_str), Some(c"notes.txt"));
    /// ```
    ///
    /// The C defects [`find_long`], [`find_short`], [`skip_choice_args`], [`take_short`] and
    /// [`reorder_gnu_style`] list are not carried over: a parser that has no table, as a zeroed
    /// one, finds no spec, and its arguments are unknown, where the C reads through a null
    /// pointer.
    ///
    /// Replaces `fn::cli_opt_parser_next`
    #[doc(alias = "cli_opt_parser_next")]
    #[must_use]
    pub fn parse_next(&mut self) -> Opt<'p> {
        loop {
            let state = self.as_ref();
            let Some(arg) = state.args().get(state.idx()).copied() else {
                let mut done = Opt::zeroed();
                done.as_mut().set_args_len(state.in_args());
                return done;
            };
            let bytes = arg.as_c_str().to_bytes();
            let (in_short, in_literal) = (state.in_short() != 0, state.in_literal());
            if bytes.starts_with(b"--") && !in_short && !in_literal {
                return take_long(self);
            }
            if in_short || (bytes.starts_with(b"-") && !in_literal) {
                return take_short(self);
            }
            if !(state.needs_sort() && reorder_gnu_style(self)) {
                return take_bare(self);
            }
        }
    }
}

/// Parses every argument of `args` against `specs` under `flags`, as a [`Parser`] does, and
/// checks that each required spec ([`Usage::REQUIRED`]) was given, or one option of a required
/// choice: the [`Status::Done`] outcome and the arguments in the order the parse left them, which
/// a GNU-style parse may have changed, and in which a [`Kind::Args`] spec's [`ArgsSlot`] finds what
/// it took. The specs' variables hold what the arguments gave them.
///
/// An argument that ends parsing ([`Usage::STOP_PARSING`]), as a command's `--help`, ends it
/// there, unchecked: the outcome is that argument's, its status [`Status::Done`].
///
/// ```
/// use core::cell::Cell;
///
/// use libgit2_crustify::git2::src::cli::opt::{
///     Arg, ArgsSlot, ParseFlags, Spec, SpecTable, Status, Target, Usage, parse,
/// };
///
/// let (quiet, paths) = (Cell::new(0), ArgsSlot::new());
/// let specs = [
///     Spec::new(Target::Bool(Some(&quiet))).with_alias(b'q'),
///     Spec::new(Target::Args(Some(&paths))).with_usage(Usage::REQUIRED),
///     Spec::default(),
/// ];
/// let table = SpecTable::new(&specs).unwrap();
/// let mut args = [c"a.txt", c"-q", c"b.txt"].map(Arg::new);
/// let (done, args) = parse(table, &mut args, ParseFlags::FORCE_GNU).unwrap();
/// assert_eq!(done.as_ref().args_len(), 2);
/// let taken: Vec<_> = paths
///     .get_in(args)
///     .unwrap()
///     .iter()
///     .map(|a| a.as_c_str())
///     .collect();
/// assert_eq!(taken, [c"a.txt", c"b.txt"]);
/// assert_eq!(quiet.get(), 1);
/// ```
///
/// The parse borrows the arguments for as long as the variables it wrote, which hold them, are
/// in use, so they cannot be changed meanwhile; the arguments it hands back are that borrow's:
///
/// ```compile_fail,E0506
/// use core::cell::Cell;
///
/// use libgit2_crustify::git2::src::cli::opt::{Arg, ParseFlags, Spec, SpecTable, Target, parse};
///
/// let file = Cell::new(None);
/// let specs = [Spec::new(Target::Arg(Some(&file))), Spec::default()];
/// let table = SpecTable::new(&specs).unwrap();
/// let mut args = [Arg::new(c"a")];
/// let _ = parse(table, &mut args, ParseFlags::DEFAULT);
/// args[0] = Arg::new(c"b");
/// let _ = file.get();
/// ```
///
/// # Errors
///
/// The outcome of the first argument that fails, an [`Status::UnknownOption`] or a
/// [`Status::MissingValue`], at which the parse stops; or, once every argument is parsed, a
/// [`Status::MissingArgument`] outcome whose spec is the first requirement not given, the first
/// spec of its choice.
///
/// The C keeps the specs the arguments gave in a stack array of one entry per argument, and a
/// bundle of short options gives one per option, so that `-ab` writes past it: here they are
/// kept in a [`Vec`], with no bound.
///
/// Replaces `fn::cli_opt_parse`
#[doc(alias = "cli_opt_parse")]
pub fn parse<'p>(
    specs: SpecTable<'p>,
    args: &'p mut [Arg<'p>],
    flags: ParseFlags,
) -> Result<(Opt<'p>, &'p [Arg<'p>]), Opt<'p>> {
    let mut parser = Parser::new(specs, args, flags);
    let mut given = Vec::new();
    let (mut outcome, stopped) = loop {
        let mut opt = parser.as_mut().parse_next();
        match opt.as_ref().status() {
            Some(Status::Ok) => {}
            Some(Status::Done) => break (opt, false),
            _ => return Err(opt),
        }
        if let Some(spec) = opt.as_ref().spec() {
            if spec.usage().contains(Usage::STOP_PARSING) {
                opt.as_mut().set_status(Status::Done);
                break (opt, true);
            }
            given.push(spec);
        }
    };
    if !stopped && let Err(missing) = check_required(specs, &given) {
        let mut out = outcome.as_mut();
        out.set_spec(Some(missing));
        out.set_status(Status::MissingArgument);
        return Err(outcome);
    }
    Ok((outcome, parser.into_args()))
}

/// Parses every argument of `args` against `specs` under `flags`, as a [`Parser`] does, and calls
/// `callback` with each outcome, failures included, until the arguments are done or it breaks:
/// what it broke with, or [`ControlFlow::Continue`] once every argument is parsed. Nothing is
/// checked of the specs required.
///
/// ```
/// use core::ffi::CStr;
/// use core::ops::ControlFlow;
///
/// use libgit2_crustify::git2::src::cli::opt::{
///     Arg, ForeachCallbackFn, OptRef, ParseFlags, Spec, SpecTable, Target, for_each,
/// };
///
/// let specs = [
///     Spec::new(Target::Value(None)).with_alias(b'c'),
///     Spec::default(),
/// ];
/// let table = SpecTable::new(&specs).unwrap();
/// let mut args = [c"-cuser.name=me", c"log", c"-c", c"core.pager=less"].map(Arg::new);
/// let mut config = Vec::new();
/// let mut keep = |opt: OptRef<'_, '_>| {
///     config.extend(opt.value().map(CStr::to_owned));
///     ControlFlow::Continue(())
/// };
/// let flow = for_each(
///     table,
///     &mut args,
///     ParseFlags::FORCE_GNU,
///     ForeachCallbackFn::new(&mut keep),
/// );
/// assert_eq!(flow, ControlFlow::Continue(()));
/// assert_eq!(config, [c"user.name=me", c"core.pager=less"]);
/// ```
///
/// Replaces `fn::cli_opt_foreach`
#[doc(alias = "cli_opt_foreach")]
pub fn for_each<'p>(
    specs: SpecTable<'p>,
    args: &'p mut [Arg<'p>],
    flags: ParseFlags,
    mut callback: ForeachCallbackFn<'_, 'p>,
) -> ControlFlow<NonZero<c_int>> {
    let mut parser = Parser::new(specs, args, flags);
    loop {
        let opt = parser.as_mut().parse_next();
        if opt.as_ref().status() == Some(Status::Done) {
            return ControlFlow::Continue(());
        }
        callback.call(opt.as_ref())?;
    }
}

/// What C's `printf` writes for a `NULL` string, which glibc prints as `(null)`.
const NULL_TEXT: &[u8] = b"(null)";

/// The bytes of a string a spec or an outcome holds, glibc's `(null)` for none.
fn c_text(s: Option<&CStr>) -> &[u8] {
    s.map_or(NULL_TEXT, CStr::to_bytes)
}

/// Writes the name a status message gives `spec`, in one write: its value name for a
/// [`Kind::Arg`] or [`Kind::Args`] spec, as `<path>`; else `-` and its alias, unless it has none
/// or is shown long ([`Usage::SHOW_LONG`]); else `--` and its long name. A name or value name it
/// lacks is written `(null)`, as glibc's `printf` writes a `NULL` string.
///
/// Replaces `fn::spec_name_fprint`
#[doc(alias = "spec_name_fprint")]
fn write_spec_name<W: io::Write + ?Sized>(out: &mut W, spec: SpecRef<'_, '_>) -> io::Result<()> {
    match spec.kind() {
        Some(Kind::Arg | Kind::Args) => out.write_all(c_text(spec.value_name())),
        _ => match spec.alias() {
            Some(alias) if !spec.usage().contains(Usage::SHOW_LONG) => {
                out.write_all(&[b'-', alias.get()])
            }
            _ => out.write_all(&[b"--", c_text(spec.name())].concat()),
        },
    }
}

impl<'p> OptRef<'_, 'p> {
    /// Writes to `out` the message the CLI shows for this outcome, after `command` and `": "`
    /// when given:
    ///
    /// - [`Status::Done`]: `finished processing arguments (no error)`;
    /// - [`Status::Ok`]: `no error`;
    /// - [`Status::UnknownOption`]: `unknown option: ` and the argument;
    /// - [`Status::MissingValue`]: `argument '<name>' requires a value.`;
    /// - [`Status::MissingArgument`]: `argument '<name>' is required.`, or, for a spec that starts
    ///   a choice, `either '<a>' or '<b>' is required.` and `one of '<a>', '<b>' or '<c>' is
    ///   required.`, each choice named in order; and
    /// - a status `opt.h` does not name, which only an outcome C wrote holds: `unknown status: `
    ///   and its value, as a C `int`;
    ///
    /// each ended by a newline. An option is named by its alias, as `-f`, or by its long name, as
    /// `--foo`, when it has no alias or is shown long ([`Usage::SHOW_LONG`]), and a bare argument
    /// by its value name. The argument and the names are written as their bytes, and one that is
    /// missing as `(null)`, as glibc's `printf` writes a `NULL` string.
    ///
    /// `specs` is the table the outcome's spec is in, which a choice's later options are read
    /// from: a spec it does not hold is named alone. The message is written in the pieces C's
    /// `fprintf` calls write, each in one [`write_all`](io::Write::write_all), and stops at the
    /// first that fails, as the C does.
    ///
    /// ```
    /// use libgit2_crustify::git2::src::cli::opt::{Opt, Spec, SpecTable, Status, Target, Usage};
    ///
    /// let specs = [
    ///     Spec::new(Target::Switch(None)).with_alias(b'f'),
    ///     Spec::new(Target::Switch(None))
    ///         .with_name(c"bar")
    ///         .with_usage(Usage::CHOICE),
    ///     Spec::default(),
    /// ];
    /// let table = SpecTable::new(&specs).unwrap();
    /// let mut opt = Opt::default();
    /// opt.as_mut().set_status(Status::MissingArgument);
    /// opt.as_mut().set_spec(table.get(0));
    /// let mut out = Vec::new();
    /// opt.as_ref()
    ///     .write_status(&mut out, Some("git2"), table)
    ///     .unwrap();
    /// assert_eq!(out, b"git2: either '-f' or '--bar' is required.\n");
    /// ```
    ///
    /// # Errors
    ///
    /// The first error of `out`'s writes, after the pieces before it are written; and
    /// [`io::ErrorKind::InvalidInput`], with nothing written, for a [`Status::MissingValue`] or
    /// [`Status::MissingArgument`] outcome that names no spec, which the C reads through a null
    /// pointer.
    ///
    /// The C returns the count of the last piece it writes. `opt.h` documents 0 on success, which
    /// the export returns, and `-1` on failure.
    ///
    /// Replaces `fn::cli_opt_status_fprint`
    #[doc(alias = "cli_opt_status_fprint")]
    pub fn write_status<W: io::Write + ?Sized>(
        &self,
        out: &mut W,
        command: Option<&str>,
        specs: SpecTable<'p>,
    ) -> io::Result<()> {
        self.write_status_bytes(out, command.map(str::as_bytes), specs)
    }

    /// [`write_status`](Self::write_status) after a `command` of any bytes, as C's `%s` writes the
    /// C string it is given: the export's, whose `command` need not be UTF-8.
    fn write_status_bytes<W: io::Write + ?Sized>(
        self,
        out: &mut W,
        command: Option<&[u8]>,
        specs: SpecTable<'p>,
    ) -> io::Result<()> {
        let message = match (self.status(), self.spec()) {
            (Some(Status::Done), _) => Message::Done,
            (Some(Status::Ok), _) => Message::Ok,
            (Some(Status::UnknownOption), _) => Message::UnknownOption,
            (Some(Status::MissingValue), Some(spec)) => Message::MissingValue(spec),
            (Some(Status::MissingArgument), Some(spec)) => Message::MissingArgument(spec),
            (Some(Status::MissingValue | Status::MissingArgument), None) => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "the outcome names no spec",
                ));
            }
            (None, _) => Message::Unnamed(c_int::from_ne_bytes(self.raw_status().to_ne_bytes())),
        };
        if let Some(command) = command {
            out.write_all(&[command, b": "].concat())?;
        }
        match message {
            Message::Done => out.write_all(b"finished processing arguments (no error)\n"),
            Message::Ok => out.write_all(b"no error\n"),
            Message::UnknownOption => {
                out.write_all(&[b"unknown option: ", c_text(self.arg()), b"\n"].concat())
            }
            Message::MissingValue(spec) => {
                out.write_all(b"argument '")?;
                write_spec_name(out, spec)?;
                out.write_all(b"' requires a value.\n")
            }
            Message::MissingArgument(spec) => {
                if let Some(first) = specs.position(spec).filter(|&at| is_choice(specs, at)) {
                    write_choices(out, specs, first)
                } else {
                    out.write_all(b"argument '")?;
                    write_spec_name(out, spec)?;
                    out.write_all(b"' is required.\n")
                }
            }
            Message::Unnamed(raw) => {
                out.write_all(&[b"unknown status: ", raw.to_string().as_bytes(), b"\n"].concat())
            }
        }
    }
}

/// The message [`OptRef::write_status`] writes, decided before any of it is.
enum Message<'s> {
    /// [`Status::Done`].
    Done,
    /// [`Status::Ok`].
    Ok,
    /// [`Status::UnknownOption`], naming the argument.
    UnknownOption,
    /// [`Status::MissingValue`] of the spec.
    MissingValue(SpecRef<'s, 's>),
    /// [`Status::MissingArgument`] of the spec, or of the choice it starts.
    MissingArgument(SpecRef<'s, 's>),
    /// A status `opt.h` does not name, as a C `int`.
    Unnamed(c_int),
}

/// Writes the requirement of the choice that starts at `first` of `specs`, which
/// [`is_choice`]: `either` its two options, or `one of` its three or more, each named, and
/// `is required.`.
fn write_choices<W: io::Write + ?Sized>(
    out: &mut W,
    specs: SpecTable<'_>,
    first: usize,
) -> io::Result<()> {
    let lead: &[u8] = if is_choice(specs, first + 1) {
        b"one of"
    } else {
        b"either"
    };
    out.write_all(lead)?;
    let mut index = first;
    while let Some(choice) = specs.get(index) {
        let more = is_choice(specs, index);
        if !more {
            out.write_all(b" or")?;
        } else if index != first {
            out.write_all(b",")?;
        }
        out.write_all(b" '")?;
        write_spec_name(out, choice)?;
        out.write_all(b"'")?;
        if !more {
            break;
        }
        index += 1;
    }
    out.write_all(b" is required.\n")
}

/// A C stream as an [`io::Write`], each write one locked `fwrite` of its bytes as they are
/// ([`FileRef::write_all`](glibc_bindings::libc::libio::genops::FileRef::write_all)), as C's
/// `fprintf` calls write the status message's pieces, in order with the C that shares the stream.
///
/// A failed write is not retried, as C's `fprintf` is not: its [`write_all`](io::Write::write_all)
/// is that one `fwrite`. The trait's own `write_all` writes the whole of what is left again after
/// an [`io::ErrorKind::Interrupted`] failure, which an `EINTR` part way through the bytes reports,
/// so that the bytes already out would be written twice, and the message reported written, where
/// C's `fprintf` fails. For the same reason [`write`](io::Write::write) fails whenever the `fwrite`
/// does, though part of `buf` may have reached the stream: `fwrite`'s count is not reported, and
/// the trait's documentation, that a failed `write` wrote nothing, does not hold of it. The export
/// writes only through `write_all`.
#[cfg(any(test, feature = "ffi-export-src-cli-opt-c"))]
struct FileWriter<'a>(glibc_bindings::libc::libio::genops::FileRef<'a>);

#[cfg(any(test, feature = "ffi-export-src-cli-opt-c"))]
impl io::Write for FileWriter<'_> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.write_all(buf)?;
        Ok(buf.len())
    }

    fn write_all(&mut self, buf: &[u8]) -> io::Result<()> {
        self.0.write_all(buf)
    }

    /// Nothing: C's `cli_opt_status_fprint` leaves the stream's buffer to its buffering mode.
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// The CLI's three structs as C lays them out, with their handles and accessors, the argument
/// strings they borrow, and the C form of the callback a whole parse calls.
#[expect(
    unsafe_code,
    reason = "the C layouts of opt.h's structs, the argument strings they point at, and the C \
              functions a parse's callback calls"
)]
mod ffi_import {
    use core::cell::Cell;
    use core::ffi::{CStr, c_char, c_int, c_uint, c_void};
    use core::fmt;
    use core::marker::PhantomData;
    use core::num::NonZero;
    use core::ops::ControlFlow;
    use core::ptr::{self, NonNull};

    use ffibox::callback::CFn;
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

        /// The table C passes at `p`, a `const cli_opt_spec[]` ended by its first spec of kind
        /// `CLI_OPT_TYPE_NONE`, or `None` for `NULL`. Its specs are read up to that end, which
        /// it holds after them; `p` may address any spec of an array, the table then starting
        /// there.
        ///
        /// The adoption through which an export reaches the spec tables of the CLI's remaining
        /// C, as [`CRefSeam::from_ptr`] adopts one spec.
        ///
        /// # Safety
        ///
        /// `p` is `NULL`, or addresses, with provenance over all of it, an array of
        /// `cli_opt_spec`s ended by one of kind
        /// `CLI_OPT_TYPE_NONE`, that lives, unchanged, for `'p`, and each spec before that end
        /// is a valid `Spec<'p>`, as [`Spec`]'s adoption asks: its strings are `NULL` or live,
        /// unchanged, for `'p`, and its `value` is `NULL` or the variable its `type` writes,
        /// live for `'p` and written only by a parse.
        #[must_use]
        pub unsafe fn from_ptr(p: *const sys::cli_opt_spec) -> Option<Self> {
            let p = NonNull::new(p.cast_mut())?;
            // SAFETY: the caller's contract, for a non-null `p`.
            Some(unsafe { Self::from_c(p) })
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
            Status::from_raw(self.raw_status())
        }

        /// The outcome as C holds it, a value `opt.h` does not name included.
        pub(super) fn raw_status(&self) -> u32 {
            let p = self.as_ptr();
            // SAFETY: `p` is the handle's pointer to the live outcome it borrows; the
            // projection forms no reference to it.
            let status_ptr = unsafe { &raw const (*p).status };
            // SAFETY: a read through the shared handle's pointer to a live outcome.
            unsafe { status_ptr.read() }
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
        /// The arguments the parser read, in the order it left them, given back for `'p` once
        /// the parser is done with: the array it borrowed exclusively for `'p`, which nothing
        /// but the parser reached meanwhile.
        pub(super) fn into_args(self) -> &'p mut [Arg<'p>] {
            let p = self.as_ref().as_ptr();
            // SAFETY: `p` is the pointer of the parser `self` holds; the projection forms no
            // reference to it.
            let args_ptr = unsafe { &raw const (*p).args };
            // SAFETY: as above.
            let len_ptr = unsafe { &raw const (*p).args_len };
            // SAFETY: a read of the live parser `self` holds.
            let args = unsafe { args_ptr.read() };
            // SAFETY: as above.
            let len = unsafe { len_ptr.read() };
            if args.is_null() || len == 0 {
                return &mut [];
            }
            // SAFETY: `new` and `set_args` store an `&'p mut [Arg<'p>]`'s start and length, the
            // only ways a `Parser<'p>` value comes to hold an array, so `args` addresses `len`
            // valid `Arg<'p>`s borrowed exclusively for `'p`. That borrow was the parser's, which
            // this consumes: no handle of it, which would borrow it, lives, so nothing else
            // reaches the array for `'p`.
            unsafe { core::slice::from_raw_parts_mut(args.cast::<Arg<'p>>(), len) }
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

    /// The C form of a [`ForeachCallbackFn`]: the type of `cli_opt_foreach`'s `callback`, which
    /// C calls with each outcome and the `callback_data` it was given beside it.
    pub(crate) type ForeachCallbackC =
        unsafe extern "C" fn(*mut sys::cli_opt, *mut c_void) -> c_int;

    // The C form is the type `cli_opt_foreach` declares for its callback: the function's raw
    // binding coerces to a function pointer that takes it only when the two agree.
    const _: unsafe extern "C" fn(
        *const sys::cli_opt_spec,
        *mut *mut c_char,
        usize,
        c_uint,
        Option<ForeachCallbackC>,
        *mut c_void,
    ) -> c_int = sys::cli_opt_foreach;

    /// A closure called with each outcome.
    type ForeachClosure<'f, 'p> = dyn FnMut(OptRef<'_, 'p>) -> ControlFlow<NonZero<c_int>> + 'f;

    /// The callback a whole parse calls with the outcome of each argument, which stops the
    /// parse by breaking with a code, the parse's result; `cli_opt_foreach`'s `callback`, with
    /// its `callback_data`.
    ///
    /// A Rust closure, borrowed for `'f`, or a C function with its context, which the CLI's
    /// remaining C (`common.c`'s `parse_option`) hands the parse's export: called the same way
    /// whichever it is. The parse lends each outcome for the call alone, and the outcome lends
    /// its spec, argument and value for `'p`, the borrow of the parse's table and arguments, so
    /// a closure may keep those, but not the outcome:
    ///
    /// ```
    /// use core::ffi::CStr;
    /// use core::ops::ControlFlow;
    ///
    /// use libgit2_crustify::git2::src::cli::opt::{ForeachCallbackFn, Opt, OptRef};
    ///
    /// let mut values: Vec<Option<&CStr>> = Vec::new();
    /// let mut keep = |opt: OptRef<'_, 'static>| {
    ///     values.push(opt.value());
    ///     ControlFlow::Continue(())
    /// };
    /// let mut callback = ForeachCallbackFn::new(&mut keep);
    /// let mut opt = Opt::default();
    /// opt.as_mut().set_value(Some(c"x"));
    /// assert_eq!(callback.call(opt.as_ref()), ControlFlow::Continue(()));
    /// assert_eq!(values, [Some(c"x")]);
    /// ```
    ///
    /// ```compile_fail,E0521
    /// use core::ops::ControlFlow;
    /// use libgit2_crustify::git2::src::cli::opt::{ForeachCallbackFn, OptRef};
    ///
    /// fn keep<'k, 'p>(kept: &mut Option<OptRef<'k, 'p>>) {
    ///     let mut f = |opt: OptRef<'_, 'p>| {
    ///         *kept = Some(opt);
    ///         ControlFlow::Continue(())
    ///     };
    ///     let _ = ForeachCallbackFn::new(&mut f);
    /// }
    /// ```
    ///
    /// The parse calls it on the thread that holds it, and the closure need not be `Send`; nor
    /// is the callback:
    ///
    /// ```compile_fail,E0277
    /// use core::ops::ControlFlow;
    /// use libgit2_crustify::git2::src::cli::opt::{ForeachCallbackFn, OptRef};
    ///
    /// fn send<T: Send>(_: T) {}
    /// let mut f = |_: OptRef<'_, '_>| ControlFlow::Continue(());
    /// send(ForeachCallbackFn::new(&mut f));
    /// ```
    ///
    /// Replaces `fnp::cli_opt_foreach.callback`
    #[doc(alias = "cli_opt_foreach.callback")]
    pub struct ForeachCallbackFn<'f, 'p>(ForeachInner<'f, 'p>);

    /// The two implementations of a [`ForeachCallbackFn`].
    enum ForeachInner<'f, 'p> {
        /// A Rust closure.
        Rust(&'f mut ForeachClosure<'f, 'p>),
        /// A C function and its context, callable for `'f`.
        C(CFn<'f, ForeachCallbackC, 1>),
    }

    impl<'f, 'p> ForeachCallbackFn<'f, 'p> {
        /// The callback that calls `f` with each outcome, until it breaks.
        pub fn new<F>(f: &'f mut F) -> Self
        where
            F: FnMut(OptRef<'_, 'p>) -> ControlFlow<NonZero<c_int>> + 'f,
        {
            Self(ForeachInner::Rust(f))
        }

        /// The callback that calls the C function `f` with each outcome and `ctx`, until it
        /// returns other than 0, as `cli_opt_foreach` calls the function it is given.
        ///
        /// # Safety
        ///
        /// `f` may be called with `ctx` as its second argument, any number of times, on the
        /// calling thread, until `'f` ends; each call passes it an outcome that lends what it
        /// points at for `'p`, which `f` only reads, and whose address it does not keep once it
        /// returns, as `common.c`'s `parse_option` does.
        #[cfg_attr(
            not(any(test, feature = "ffi-export-src-cli-opt-c")),
            expect(dead_code, reason = "the export of `cli_opt_foreach` calls it")
        )]
        pub(crate) unsafe fn from_c(f: ForeachCallbackC, ctx: *mut c_void) -> Self {
            // SAFETY: the caller's contract, for `'f`.
            Self(ForeachInner::C(unsafe { CFn::new(f, ctx) }))
        }

        /// Calls the callback with `opt`, the outcome of one argument: whether the parse goes
        /// on, or stops with the code the callback breaks with. A C function's return is the
        /// code, 0 going on.
        pub fn call(&mut self, opt: OptRef<'_, 'p>) -> ControlFlow<NonZero<c_int>> {
            match &mut self.0 {
                ForeachInner::Rust(f) => f(opt),
                ForeachInner::C(f) => {
                    // SAFETY: `from_c`'s contract makes `f` callable with its context now, on
                    // this thread; `opt` is a live outcome that lends what it points at for
                    // `'p`, which `f` only reads during the call, so the pointer's write
                    // permission is never used.
                    let rc = unsafe { f.call((opt.as_ptr().cast_mut(),)) };
                    NonZero::new(rc).map_or(ControlFlow::Continue(()), ControlFlow::Break)
                }
            }
        }
    }

    impl fmt::Debug for ForeachCallbackFn<'_, '_> {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            match &self.0 {
                ForeachInner::Rust(_) => f.write_str("ForeachCallbackFn(Rust)"),
                ForeachInner::C(c) => f.debug_tuple("ForeachCallbackFn").field(c).finish(),
            }
        }
    }

    /// The C form of [`ForeachCallbackFn`], over a C-ABI function standing in for `common.c`'s:
    /// it is called with the outcome and its context, and its return is the code it breaks with.
    #[cfg(test)]
    mod unit_priv {
        use core::ffi::{c_int, c_void};
        use core::ops::ControlFlow;
        use core::ptr;

        use ffibox::{CRefSeam, CZeroable};

        use super::{ForeachCallbackFn, Opt, Status};
        use crate::sys::git2 as sys;

        /// What [`record`] saw, and when it breaks.
        struct Seen {
            outcomes: Vec<(*const sys::cli_opt, u32)>,
            break_at: usize,
        }

        /// Records the outcome it is given and its status in the `Seen` at `ctx`, and returns 7
        /// once it has seen `break_at` of them, 0 before.
        unsafe extern "C" fn record(opt: *mut sys::cli_opt, ctx: *mut c_void) -> c_int {
            // SAFETY: `ctx` is the test's `Seen`, which nothing else uses during the call.
            let seen = unsafe { &mut *ctx.cast::<Seen>() };
            // SAFETY: `opt` is the live outcome the callback lends for the call.
            let status = unsafe { (*opt).status };
            seen.outcomes.push((opt.cast_const(), status));
            if seen.outcomes.len() == seen.break_at {
                7
            } else {
                0
            }
        }

        #[test]
        fn a_c_function_is_called_with_the_outcome_and_its_context() {
            let mut seen = Seen {
                outcomes: Vec::new(),
                break_at: 2,
            };
            let mut opt = Opt::zeroed();
            opt.as_mut().set_status(Status::Ok);
            {
                // SAFETY: `record` reads the outcome alone, and `seen` outlives the callback,
                // which nothing else touches while it lives.
                let mut callback =
                    unsafe { ForeachCallbackFn::from_c(record, ptr::from_mut(&mut seen).cast()) };
                assert_eq!(callback.call(opt.as_ref()), ControlFlow::Continue(()));
                opt.as_mut().set_status(Status::MissingValue);
                assert_eq!(
                    callback.call(opt.as_ref()),
                    ControlFlow::Break(core::num::NonZero::new(7).unwrap())
                );
                assert!(format!("{callback:?}").starts_with("ForeachCallbackFn(CFn"));
            }
            let at = opt.as_ref().as_ptr();
            assert_eq!(
                seen.outcomes,
                [
                    (at, sys::CLI_OPT_STATUS_OK),
                    (at, sys::CLI_OPT_STATUS_MISSING_VALUE)
                ]
            );
        }

        #[test]
        fn a_closure_is_shown_as_rust() {
            let mut f = |_: super::OptRef<'_, '_>| ControlFlow::Continue(());
            let callback = ForeachCallbackFn::new(&mut f);
            assert_eq!(format!("{callback:?}"), "ForeachCallbackFn(Rust)");
        }
    }

    /// The parser's internal routines against C's: `opt.c`'s static functions are reached
    /// through the exported routines that call them, `cli_opt_parser_next`, `cli_opt_parse` and
    /// `cli_opt_foreach`, over tables built through the safe API, and the stream the export writes
    /// through against C's own `fprintf` calls, into memory streams. All of them call C.
    #[cfg(test)]
    mod equiv_priv {
        use core::cell::Cell;
        use core::ffi::{CStr, c_char, c_int, c_void};
        use core::ops::ControlFlow;
        use core::ptr;

        use ffibox::{CMutSeam, CRefSeam, CZeroable};
        use glibc_bindings::libc::ffi as libc;
        use glibc_bindings::libc::libio::genops::FileRef;

        use super::super::{
            FileWriter, LongForm, check_required, find_arg, find_long, find_short, skip_choice_args,
        };
        use super::{
            Arg, ForeachCallbackFn, Opt, ParseFlags, Parser, Spec, SpecRef, SpecTable, Status,
            Target, Usage,
        };
        use crate::sys::git2 as sys;

        /// One step of C's parser, `cli_opt_parser_next`.
        fn c_next<'p>(opt: &mut Opt<'p>, parser: &mut Parser<'p>) -> Option<Status> {
            // SAFETY: both are live values Rust holds, borrowed exclusively for the call; the
            // parser's table and arguments are borrowed for `'p`, so what C stores in the outcome
            // and the specs' variables lives as long as they do.
            let raw = unsafe {
                sys::cli_opt_parser_next(opt.as_mut().as_mut_ptr(), parser.as_mut().as_mut_ptr())
            };
            Status::from_raw(raw)
        }

        /// The first outcome C's parser gives `arg` alone, against `table`, and the parser after
        /// it.
        fn c_first<'p>(table: SpecTable<'p>, args: &'p mut [Arg<'p>]) -> (Opt<'p>, Parser<'p>) {
            let mut parser = Parser::new(table, args, ParseFlags::DEFAULT);
            let mut opt = Opt::zeroed();
            c_next(&mut opt, &mut parser);
            (opt, parser)
        }

        /// Where a spec C or Rust found is in `table`.
        fn index(table: SpecTable<'_>, spec: Option<SpecRef<'_, '_>>) -> Option<usize> {
            spec.map(|s| {
                let offset = s.as_ptr().addr() - table.get(0).unwrap().as_ptr().addr();
                offset / size_of::<Spec<'_>>()
            })
        }

        /// `spec_for_long` in `parse_long`: the spec C's parser selects for `--<arg>`, the value it
        /// takes from after the `=`, and the bool it sets to 0 for `no-`, against [`find_long`].
        #[test]
        #[cfg_attr(miri, ignore = "calls C")]
        fn long_arguments_select_the_specs_c_selects() {
            let args: [&CStr; 22] = [
                c"debug",
                c"no-debug",
                c"read",
                c"no-read",
                c"verbose",
                c"file",
                c"output",
                c"output=x",
                c"output=",
                c"output=a=b",
                c"",
                c"outputs=x",
                c"x",
                c"no-x",
                c"unknown",
                c"o=1",
                c"o",
                c"no-",
                c"=",
                c"=v",
                c"no-x=1",
                c"no-debugx",
            ];
            for arg in args {
                let (debug, x) = (Cell::new(7), Cell::new(7));
                let specs = [
                    Spec::new(Target::Bool(Some(&debug))).with_name(c"debug"),
                    Spec::new(Target::Switch(None)).with_name(c"read"),
                    Spec::new(Target::Accumulator(None)).with_name(c"verbose"),
                    Spec::new(Target::Value(None)).with_name(c"output"),
                    Spec::new(Target::Literal),
                    Spec::new(Target::Value(None)).with_name(c"no-read"),
                    Spec::new(Target::Arg(None)).with_name(c"file"),
                    Spec::new(Target::Value(None)).with_name(c"o"),
                    Spec::new(Target::Bool(Some(&x))).with_name(c"x"),
                    Spec::new(Target::Value(None)).with_name(c""),
                    Spec::default(),
                ];
                let table = SpecTable::new(&specs).unwrap();
                let long = [b"--", arg.to_bytes_with_nul()].concat();
                let long = CStr::from_bytes_with_nul(&long).unwrap();
                let mut c_args = [Arg::new(long)];
                debug.set(7);
                x.set(7);
                let (opt, parser) = c_first(table, &mut c_args);
                let o = opt.as_ref();
                let rust = find_long(parser.as_ref(), arg);
                let at = index(table, rust.map(|(s, _)| s));
                assert_eq!(index(table, o.spec()), at, "{arg:?}");
                // What C wrote to the bool the argument selected, if it selected one.
                let bool_set = match at {
                    Some(0) => Some(debug.get()),
                    Some(8) => Some(x.get()),
                    _ => None,
                };
                match rust.map(|(_, form)| form) {
                    Some(LongForm::Valued(value)) => assert_eq!(o.value(), value, "{arg:?}"),
                    Some(LongForm::Negated) => assert_eq!(bool_set, Some(0), "{arg:?}"),
                    Some(LongForm::Plain) => {
                        assert_eq!(o.value(), None, "{arg:?}");
                        assert!(bool_set.is_none_or(|v| v == 1), "{arg:?}");
                    }
                    None => assert_eq!(o.status(), Some(Status::UnknownOption), "{arg:?}"),
                }
            }
        }

        /// `spec_for_short` in `parse_short`: the spec C's parser selects for `-<arg>` and the
        /// value it takes from the rest of the argument, against [`find_short`]; the bare
        /// `-`, where the C reads past the argument, is not compared.
        #[test]
        #[cfg_attr(miri, ignore = "calls C")]
        fn short_options_select_the_specs_c_selects() {
            let args: [&CStr; 11] = [
                c"ofile", c"o", c"o-", c"d", c"dvv", c"vd", c"a", c"x", c"\xe9", c"l", c"h",
            ];
            for arg in args {
                let specs = [
                    Spec::new(Target::Value(None)).with_alias(b'o'),
                    Spec::new(Target::Bool(None)).with_alias(b'd'),
                    Spec::new(Target::Accumulator(None)).with_alias(b'v'),
                    Spec::new(Target::Arg(None)).with_alias(b'a'),
                    Spec::new(Target::Switch(None)).with_alias(b'd'),
                    Spec::new(Target::Switch(None)).with_name(c"help"),
                    Spec::new(Target::Switch(None)).with_alias(0xE9),
                    Spec::new(Target::Literal).with_alias(b'l'),
                    Spec::default(),
                ];
                let table = SpecTable::new(&specs).unwrap();
                let short = [b"-", arg.to_bytes_with_nul()].concat();
                let short = CStr::from_bytes_with_nul(&short).unwrap();
                let mut c_args = [Arg::new(short)];
                let (opt, parser) = c_first(table, &mut c_args);
                let o = opt.as_ref();
                let rust = find_short(parser.as_ref(), arg);
                assert_eq!(
                    index(table, o.spec()),
                    index(table, rust.map(|(s, _)| s)),
                    "{arg:?}"
                );
                assert_eq!(o.value(), rust.and_then(|(_, v)| v), "{arg:?}");
            }
        }

        /// `spec_for_arg` in `parse_arg`: the specs C's parser gives successive bare arguments, and
        /// the argument specs it counts, against [`find_arg`].
        #[test]
        #[cfg_attr(miri, ignore = "calls C")]
        fn bare_arguments_take_the_specs_c_gives_them() {
            for shape in 0..3 {
                let specs: Vec<Spec<'_>> = match shape {
                    0 => vec![
                        Spec::new(Target::Arg(None)),
                        Spec::new(Target::Bool(None)).with_alias(b'd'),
                        Spec::new(Target::Arg(None)),
                        Spec::new(Target::Args(None)),
                        Spec::default(),
                    ],
                    1 => vec![Spec::new(Target::Arg(None)), Spec::default()],
                    _ => vec![
                        Spec::new(Target::Args(None)),
                        Spec::new(Target::Arg(None)),
                        Spec::default(),
                    ],
                };
                let table = SpecTable::new(&specs).unwrap();
                let mut c_args = [c"a", c"b", c"c", c"d"].map(Arg::new);
                let mut parser = Parser::new(table, &mut c_args, ParseFlags::DEFAULT);
                let mut rust = Parser::new(table, &mut [], ParseFlags::DEFAULT);
                let mut opt = Opt::zeroed();
                while c_next(&mut opt, &mut parser).is_some_and(|s| s != Status::Done) {
                    let found = find_arg(&mut rust.as_mut());
                    assert_eq!(
                        index(table, opt.as_ref().spec()),
                        index(table, found),
                        "{shape}"
                    );
                    assert_eq!(
                        parser.as_ref().arg_idx(),
                        rust.as_ref().arg_idx(),
                        "{shape}"
                    );
                }
            }
        }

        /// `consume_choices` in `parse_long` and `parse_short`: the argument specs C counts once an
        /// option of a choice is given, and the spec the next bare argument then takes, against
        /// [`skip_choice_args`] and [`find_arg`].
        #[test]
        #[cfg_attr(miri, ignore = "calls C")]
        fn a_choice_given_counts_the_arguments_c_counts() {
            let cases: [&[&CStr]; 6] = [
                &[c"-a", c"file"],
                &[c"--bee", c"file"],
                &[c"-c", c"file"],
                &[c"-vx", c"file"],
                &[c"file", c"-a", c"more"],
                &[c"-a", c"-c", c"file", c"rest"],
            ];
            for case in cases {
                let specs = [
                    Spec::new(Target::Switch(None)).with_alias(b'a'),
                    Spec::new(Target::Arg(None)).with_usage(Usage::CHOICE),
                    Spec::new(Target::Switch(None))
                        .with_name(c"bee")
                        .with_usage(Usage::CHOICE),
                    Spec::new(Target::Arg(None)).with_usage(Usage::CHOICE),
                    Spec::new(Target::Switch(None)).with_alias(b'c'),
                    Spec::new(Target::Value(None))
                        .with_alias(b'v')
                        .with_usage(Usage::CHOICE),
                    Spec::new(Target::Arg(None)),
                    Spec::new(Target::Args(None)),
                    Spec::default(),
                ];
                let table = SpecTable::new(&specs).unwrap();
                let mut c_args: Vec<_> = case.iter().copied().map(Arg::new).collect();
                let mut parser = Parser::new(table, &mut c_args, ParseFlags::DEFAULT);
                let mut rust = Parser::new(table, &mut [], ParseFlags::DEFAULT);
                let mut opt = Opt::zeroed();
                while c_next(&mut opt, &mut parser).is_some_and(|s| s != Status::Done) {
                    let spec = opt.as_ref().spec().unwrap();
                    if opt
                        .as_ref()
                        .arg()
                        .is_some_and(|a| a.to_bytes().starts_with(b"-"))
                    {
                        skip_choice_args(spec, &mut rust.as_mut());
                    } else {
                        let found = find_arg(&mut rust.as_mut());
                        assert_eq!(index(table, Some(spec)), index(table, found), "{case:?}");
                    }
                    assert_eq!(
                        parser.as_ref().arg_idx(),
                        rust.as_ref().arg_idx(),
                        "{case:?}"
                    );
                }
            }
        }

        /// `validate_required`, `spec_included` and `spec_is_choice` in `cli_opt_parse`: the
        /// requirement C reports missing, by its spec, or none, against [`check_required`] of the
        /// specs C's parser matched.
        ///
        /// No case bundles short options: `cli_opt_parse` keeps the specs given in an `alloca`
        /// array of `args_len + 1` entries, one per argument, and a bundle such as `-ab` gives
        /// one spec per option, so that the C writes past the array.
        #[test]
        #[cfg_attr(miri, ignore = "calls C")]
        fn the_requirement_c_reports_missing_is_the_one_rust_reports() {
            let cases: [&[&CStr]; 9] = [
                &[],
                &[c"-x"],
                &[c"-n1"],
                &[c"-n1", c"-a"],
                &[c"-n1", c"-b", c"-f"],
                &[c"-n1", c"-c", c"-g"],
                &[c"-f", c"-c", c"-n", c"2"],
                &[
                    c"-x", c"-a", c"-b", c"-c", c"-d", c"-e", c"-f", c"-g", c"-n3",
                ],
                &[c"-n1", c"-d", c"-e"],
            ];
            for case in cases {
                let specs = [
                    Spec::new(Target::Switch(None)).with_alias(b'x'),
                    Spec::new(Target::Value(None))
                        .with_alias(b'n')
                        .with_usage(Usage::REQUIRED),
                    Spec::new(Target::Switch(None))
                        .with_alias(b'a')
                        .with_usage(Usage::REQUIRED),
                    Spec::new(Target::Switch(None))
                        .with_alias(b'b')
                        .with_usage(Usage::CHOICE),
                    Spec::new(Target::Switch(None))
                        .with_alias(b'c')
                        .with_usage(Usage::CHOICE | Usage::REQUIRED),
                    Spec::new(Target::Switch(None)).with_alias(b'd'),
                    Spec::new(Target::Switch(None))
                        .with_alias(b'e')
                        .with_usage(Usage::CHOICE),
                    Spec::new(Target::Switch(None))
                        .with_alias(b'f')
                        .with_usage(Usage::REQUIRED),
                    Spec::new(Target::Switch(None))
                        .with_alias(b'g')
                        .with_usage(Usage::CHOICE),
                    Spec::default(),
                ];
                let table = SpecTable::new(&specs).unwrap();
                let mut c_args: Vec<_> = case.iter().copied().map(Arg::new).collect();
                let mut opt = Opt::zeroed();
                // SAFETY: the outcome is live and borrowed exclusively for the call; the table is a
                // `SpecTable`'s, ended by its zeroed spec, and the arguments a live array of C
                // strings, which a default parse never reorders.
                let status = unsafe {
                    sys::cli_opt_parse(
                        opt.as_mut().as_mut_ptr(),
                        table.get(0).unwrap().as_ptr(),
                        c_args.as_mut_ptr().cast(),
                        c_args.len(),
                        ParseFlags::DEFAULT.bits(),
                    )
                };
                let mut given_args: Vec<_> = case.iter().copied().map(Arg::new).collect();
                let mut parser = Parser::new(table, &mut given_args, ParseFlags::DEFAULT);
                let mut step = Opt::zeroed();
                let mut given = Vec::new();
                while c_next(&mut step, &mut parser).is_some_and(|s| s != Status::Done) {
                    assert_eq!(step.as_ref().status(), Some(Status::Ok), "{case:?}");
                    given.push(step.as_ref().spec().unwrap());
                }
                match check_required(table, &given) {
                    Ok(()) => assert_eq!(status, sys::CLI_OPT_STATUS_DONE, "{case:?}"),
                    Err(spec) => {
                        assert_eq!(status, sys::CLI_OPT_STATUS_MISSING_ARGUMENT, "{case:?}");
                        assert_eq!(index(table, opt.as_ref().spec()), index(table, Some(spec)));
                    }
                }
            }
        }

        /// What [`seen`] recorded of each outcome: its spec's index and its status.
        struct Seen<'t> {
            table: SpecTable<'t>,
            outcomes: Vec<(Option<usize>, u32)>,
            break_at: usize,
        }

        /// A C-ABI callback standing in for `common.c`'s: it records each outcome in the `Seen` at
        /// `ctx` and breaks with 9 at the `break_at`th.
        unsafe extern "C" fn seen(opt: *mut sys::cli_opt, ctx: *mut c_void) -> c_int {
            // SAFETY: `ctx` is the test's `Seen`, which nothing else uses during the call.
            let seen = unsafe { &mut *ctx.cast::<Seen<'_>>() };
            // SAFETY: `opt` is the live outcome the parse lends for the call.
            let raw = unsafe { opt.read() };
            let spec = (!raw.spec.is_null()).then(|| {
                // SAFETY: a non-null spec is an element of the table, as C's parser hands out.
                usize::try_from(unsafe {
                    raw.spec.offset_from(seen.table.get(0).unwrap().as_ptr())
                })
                .unwrap()
            });
            seen.outcomes.push((spec, raw.status));
            c_int::from(seen.outcomes.len() == seen.break_at) * 9
        }

        /// The callback's C form is called as C's `cli_opt_foreach` calls its callback: with each
        /// outcome in turn and its context, the parse stopping at the first code other than 0,
        /// which is its result.
        #[test]
        #[cfg_attr(miri, ignore = "calls C")]
        fn a_c_callback_is_called_as_cli_opt_foreach_calls_it() {
            for break_at in [0, 1, 3, 5] {
                let specs = [
                    Spec::new(Target::Switch(None)).with_alias(b'a'),
                    Spec::new(Target::Value(None)).with_name(c"val"),
                    Spec::new(Target::Arg(None)),
                    Spec::default(),
                ];
                let table = SpecTable::new(&specs).unwrap();
                let given = [c"-a", c"--val=1", c"--nope", c"file", c"extra"];
                let mut c_args = given.map(Arg::new);
                let mut c_seen = Seen {
                    table,
                    outcomes: Vec::new(),
                    break_at,
                };
                // SAFETY: the table is a `SpecTable`'s, the arguments a live array of C strings,
                // and `seen` reads each outcome, with `c_seen` as its context, for the call alone.
                let c_rc = unsafe {
                    sys::cli_opt_foreach(
                        table.get(0).unwrap().as_ptr(),
                        c_args.as_mut_ptr().cast(),
                        c_args.len(),
                        0,
                        Some(seen),
                        ptr::from_mut(&mut c_seen).cast(),
                    )
                };
                let mut r_args = given.map(Arg::new);
                let mut r_seen = Seen {
                    table,
                    outcomes: Vec::new(),
                    break_at,
                };
                let mut rc = 0;
                {
                    // SAFETY: `seen` only reads the outcome it is lent, and `r_seen` outlives the
                    // callback, which nothing else touches while it lives.
                    let mut callback = unsafe {
                        ForeachCallbackFn::from_c(seen, ptr::from_mut(&mut r_seen).cast())
                    };
                    let mut parser = Parser::new(table, &mut r_args, ParseFlags::DEFAULT);
                    let mut opt = Opt::zeroed();
                    while c_next(&mut opt, &mut parser).is_some_and(|s| s != Status::Done) {
                        if let ControlFlow::Break(code) = callback.call(opt.as_ref()) {
                            rc = code.get();
                            break;
                        }
                    }
                }
                assert_eq!(rc, c_rc, "{break_at}");
                assert_eq!(r_seen.outcomes, c_seen.outcomes, "{break_at}");
            }
        }

        /// A memory stream, `open_memstream`'s, and the bytes it holds once closed.
        struct MemStream {
            file: *mut libc::FILE,
            buf: Box<(*mut c_char, usize)>,
        }

        impl MemStream {
            fn open() -> Self {
                let mut buf = Box::new((ptr::null_mut(), 0));
                // SAFETY: both locations are boxed, so they stay put until `close` reads them.
                let file = unsafe { libc::open_memstream(&raw mut buf.0, &raw mut buf.1) };
                assert!(!file.is_null());
                Self { file, buf }
            }

            /// Closes the stream and returns what was written to it.
            fn close(self) -> Vec<u8> {
                // SAFETY: the stream is open, and nothing uses it after this.
                assert_eq!(unsafe { libc::fclose(self.file) }, 0);
                // SAFETY: `fclose` left the buffer holding `size` bytes, which `free` releases.
                let bytes =
                    unsafe { core::slice::from_raw_parts(self.buf.0.cast::<u8>(), self.buf.1) }
                        .to_vec();
                // SAFETY: the buffer is `open_memstream`'s, freed once, with `free`.
                unsafe { libc::free(self.buf.0.cast()) };
                bytes
            }
        }

        /// What C's `cli_opt_status_fprint` writes for `opt`, and what it returns.
        fn c_status(opt: &Opt<'_>, command: Option<&CStr>) -> (Vec<u8>, c_int) {
            let stream = MemStream::open();
            // SAFETY: an open stream, the command `NULL` or a C string, and an outcome whose spec
            // is in a live table, all live for the call.
            let rc = unsafe {
                sys::cli_opt_status_fprint(
                    stream.file,
                    command.map_or(ptr::null(), CStr::as_ptr),
                    opt.as_ref().as_ptr(),
                )
            };
            (stream.close(), rc)
        }

        /// What the export writes for `opt`: [`OptRef::write_status`](super::OptRef::write_status),
        /// after a `command` of any bytes, through a [`FileWriter`] over a C stream.
        fn rust_status<'p>(opt: &Opt<'p>, command: Option<&CStr>, table: SpecTable<'p>) -> Vec<u8> {
            let stream = MemStream::open();
            {
                // SAFETY: an open stream, which only this handle reaches until it is closed.
                let file = unsafe { FileRef::from_ptr(stream.file) }.unwrap();
                opt.as_ref()
                    .write_status_bytes(&mut FileWriter(file), command.map(CStr::to_bytes), table)
                    .unwrap();
            }
            stream.close()
        }

        /// The stream the export writes through holds what C's `fprintf` calls write, for each
        /// status of an outcome and its spec's choices, byte for byte: an argument and a command
        /// that are not UTF-8 included.
        #[test]
        #[cfg_attr(miri, ignore = "calls C")]
        fn the_export_writes_what_c_writes() {
            let specs = [
                Spec::new(Target::Switch(None)).with_alias(b'f'),
                Spec::new(Target::Switch(None))
                    .with_name(c"bar")
                    .with_usage(Usage::CHOICE),
                Spec::new(Target::Arg(None))
                    .with_value_name(c"<x>")
                    .with_usage(Usage::CHOICE),
                Spec::new(Target::Value(None)).with_name(c"out"),
                Spec::default(),
            ];
            let table = SpecTable::new(&specs).unwrap();
            let cases = [
                (Status::Done, None, None),
                (Status::Ok, Some(0), None),
                (Status::UnknownOption, None, Some(c"--nope")),
                (Status::MissingValue, Some(3), Some(c"--out")),
                (Status::MissingArgument, Some(0), None),
                (Status::MissingArgument, Some(1), None),
                (Status::MissingArgument, Some(3), None),
            ];
            for (status, spec, arg) in cases {
                let mut opt = Opt::zeroed();
                opt.as_mut().set_status(status);
                opt.as_mut().set_spec(spec.and_then(|i| table.get(i)));
                opt.as_mut().set_arg(arg);
                for command in [None, Some(c"git2")] {
                    let (c, rc) = c_status(&opt, command);
                    assert!(rc >= 0);
                    let rust = rust_status(&opt, command, table);
                    assert_eq!(rust, c, "{status:?} {spec:?} {command:?}");
                }
            }
            let mut opt = Opt::zeroed();
            opt.as_mut().set_status(Status::UnknownOption);
            opt.as_mut().set_arg(Some(c"--caf\xe9"));
            let (c, _) = c_status(&opt, None);
            assert_eq!(c, b"unknown option: --caf\xe9\n");
            assert_eq!(rust_status(&opt, None, table), c);
            let (c, _) = c_status(&opt, Some(c"g\xefe"));
            assert_eq!(c, b"g\xefe: unknown option: --caf\xe9\n");
            assert_eq!(rust_status(&opt, Some(c"g\xefe"), table), c);
        }

        /// A stream of `fopencookie`'s, unbuffered, whose first write takes three bytes and fails
        /// with `EINTR`, as a `write` a signal interrupts part way does, and whose later writes
        /// take everything: what it was given, kept until it is closed.
        struct Interrupted {
            file: *mut libc::FILE,
            /// The stream's [`Sink`], a `Box` given up while the stream may write to it.
            sink: *mut Sink,
        }

        /// What an [`Interrupted`] stream's writes were given, and whether the next one fails.
        struct Sink {
            bytes: Vec<u8>,
            fail: bool,
        }

        /// `cookie_io_functions_t`, which libc's raw bindings do not declare.
        #[repr(C)]
        struct CookieIo {
            read: Option<unsafe extern "C" fn(*mut c_void, *mut c_char, usize) -> isize>,
            write: Option<unsafe extern "C" fn(*mut c_void, *const c_char, usize) -> isize>,
            seek: Option<unsafe extern "C" fn(*mut c_void, *mut i64, c_int) -> c_int>,
            close: Option<unsafe extern "C" fn(*mut c_void) -> c_int>,
        }

        unsafe extern "C" {
            /// glibc's `fopencookie`, which libc's raw bindings do not declare.
            fn fopencookie(
                cookie: *mut c_void,
                mode: *const c_char,
                io: CookieIo,
            ) -> *mut libc::FILE;
        }

        /// The write function of an [`Interrupted`] stream.
        ///
        /// # Safety
        ///
        /// `cookie` is the stream's [`Sink`], which only the stream reaches while it is open, and
        /// `buf` addresses `size` readable bytes.
        unsafe extern "C" fn take(cookie: *mut c_void, buf: *const c_char, size: usize) -> isize {
            // SAFETY: the caller's contract: the sink is live, and reached by this call alone.
            let sink = unsafe { &mut *cookie.cast::<Sink>() };
            // SAFETY: the caller's contract: `buf` holds `size` bytes.
            let bytes = unsafe { core::slice::from_raw_parts(buf.cast::<u8>(), size) };
            let taken = if sink.fail { size.min(3) } else { size };
            sink.bytes.extend_from_slice(&bytes[..taken]);
            if taken < size {
                sink.fail = false;
                glibc_bindings::libc::csu::errno_loc::set_errno(libc::EINTR);
            }
            isize::try_from(taken).unwrap_or(isize::MAX)
        }

        impl Interrupted {
            fn open() -> Self {
                let sink = Box::into_raw(Box::new(Sink {
                    bytes: Vec::new(),
                    fail: true,
                }));
                let io = CookieIo {
                    read: None,
                    write: Some(take),
                    seek: None,
                    close: None,
                };
                // SAFETY: the sink stays put, and is reached by the stream alone, until `close`
                // takes it back; the mode is a C string.
                let file = unsafe { fopencookie(sink.cast(), c"w".as_ptr(), io) };
                assert!(!file.is_null());
                // SAFETY: an open stream nothing has written yet; `_IONBF` takes no buffer.
                let unbuffered = unsafe { libc::setvbuf(file, ptr::null_mut(), libc::_IONBF, 0) };
                assert_eq!(unbuffered, 0);
                Self { file, sink }
            }

            /// Closes the stream and returns what its writes took.
            fn close(self) -> Vec<u8> {
                // SAFETY: the stream is open, and nothing uses it after this; an unbuffered
                // stream holds nothing to flush.
                unsafe { libc::fclose(self.file) };
                // SAFETY: `open`'s box, which the closed stream no longer reaches, taken back
                // once.
                unsafe { Box::from_raw(self.sink) }.bytes
            }
        }

        /// A write that `EINTR` interrupts part way fails C's `fprintf`, which writes nothing
        /// more, and fails the export's write, which is not written again: both streams take the
        /// same three bytes, both report the failure, and `errno` holds `EINTR` after each.
        #[test]
        #[cfg_attr(miri, ignore = "calls C")]
        fn an_interrupted_write_fails_both_and_is_written_once() {
            use glibc_bindings::libc::csu::errno_loc::errno;

            let specs = [
                Spec::new(Target::Switch(None)).with_alias(b'f'),
                Spec::default(),
            ];
            let table = SpecTable::new(&specs).unwrap();
            let mut opt = Opt::zeroed();
            opt.as_mut().set_status(Status::UnknownOption);
            opt.as_mut().set_arg(Some(c"--nope"));
            for command in [None, Some(c"git2")] {
                let stream = Interrupted::open();
                // SAFETY: an open stream, the command `NULL` or a C string, and an outcome with
                // no spec, all live for the call.
                let rc = unsafe {
                    sys::cli_opt_status_fprint(
                        stream.file,
                        command.map_or(ptr::null(), CStr::as_ptr),
                        opt.as_ref().as_ptr(),
                    )
                };
                let c_errno = errno();
                let c = stream.close();
                assert!(rc < 0, "{command:?}");
                assert_eq!(c_errno, libc::EINTR);

                let stream = Interrupted::open();
                let written = {
                    // SAFETY: an open stream, which only this handle reaches until it is closed.
                    let file = unsafe { FileRef::from_ptr(stream.file) }.unwrap();
                    opt.as_ref().write_status_bytes(
                        &mut FileWriter(file),
                        command.map(CStr::to_bytes),
                        table,
                    )
                };
                let rust_errno = errno();
                let rust = stream.close();
                let e = written.unwrap_err();
                assert_eq!(e.kind(), std::io::ErrorKind::Interrupted);
                assert_eq!(rust_errno, libc::EINTR);
                assert_eq!(rust, c, "{command:?}");
                assert_eq!(c.len(), 3);
            }
        }

        /// A stream that refuses writes fails C's first `fprintf` and the export's first write
        /// alike, with `EBADF` in `errno`, and neither writes anything.
        #[test]
        #[cfg_attr(miri, ignore = "calls C")]
        fn a_stream_that_refuses_writes_fails_both() {
            use glibc_bindings::libc::csu::errno_loc::errno;

            let mut storage = [0_u8; 64];
            let mut read_only = || {
                // SAFETY: `storage` outlives the stream, which reads it alone, under mode `r`.
                let file = unsafe {
                    libc::fmemopen(storage.as_mut_ptr().cast(), storage.len(), c"r".as_ptr())
                };
                assert!(!file.is_null());
                file
            };
            let opt = Opt::zeroed();
            let file = read_only();
            // SAFETY: an open stream, the command a C string, and a zeroed outcome, live for the
            // call.
            let rc = unsafe {
                sys::cli_opt_status_fprint(file, c"git2".as_ptr(), opt.as_ref().as_ptr())
            };
            let c_errno = errno();
            // SAFETY: the stream is open, and nothing uses it after this.
            unsafe { libc::fclose(file) };
            assert!(rc < 0);
            let file = read_only();
            let written = {
                // SAFETY: an open stream, which only this handle reaches until it is closed.
                let stream = unsafe { FileRef::from_ptr(file) }.unwrap();
                opt.as_ref().write_status(
                    &mut FileWriter(stream),
                    Some("git2"),
                    SpecTable::new(&[Spec::zeroed()]).unwrap(),
                )
            };
            // SAFETY: as above.
            unsafe { libc::fclose(file) };
            let e = written.unwrap_err();
            assert_eq!(e.raw_os_error(), Some(c_errno));
            assert_eq!(e.raw_os_error(), Some(nix::errno::Errno::EBADF as c_int));
            assert!(storage.iter().all(|&b| b == 0));
        }
    }
}

/// The gateway through which the CLI's remaining C calls this module once it has switched:
/// `cli_opt_parser_init` and `cli_opt_parser_next`, which `main.c` calls, `cli_opt_parse`, which
/// each `cmd_*.c` calls, `cli_opt_foreach`, which `common.c` calls, and `cli_opt_status_fprint`,
/// which `main.c` and `opt_usage.c` call, each checked against its raw binding's signature, so
/// that a C declaration that drifts fails to build.
#[cfg(feature = "ffi-export-src-cli-opt-c")]
#[expect(
    unsafe_code,
    reason = "the C ABI exports of opt.c's entry points, which adopt the pointers C passes"
)]
mod ffi_export {
    use core::ffi::{CStr, c_char, c_int, c_uint, c_void};
    use core::ops::ControlFlow;
    use core::slice;

    use ffibox::callback::barrier;
    use ffibox::{CMutSeam, CRefSeam, CZeroable};
    use glibc_bindings::libc::csu::errno_loc::set_errno;
    use glibc_bindings::libc::libio::genops::FileRef;
    use nix::errno::Errno;

    use super::{
        Arg, FileWriter, ForeachCallbackFn, Opt, OptRef, ParseFlags, Parser, ParserMut, Spec,
        SpecTable, for_each, parse,
    };
    use crate::sys::git2 as sys;

    // Each export has its raw binding's type: the array's elements coerce to one function
    // pointer type only when their signatures agree.
    const _: () = {
        let _ = [sys::cli_opt_parser_init, cli_opt_parser_init];
        let _ = [sys::cli_opt_parser_next, cli_opt_parser_next];
        let _ = [sys::cli_opt_parse, cli_opt_parse];
        let _ = [sys::cli_opt_foreach, cli_opt_foreach];
        let _ = [sys::cli_opt_status_fprint, cli_opt_status_fprint];
    };

    /// The `args_len` arguments at `args`, as C passes a parse its `argv`, borrowed exclusively
    /// for `'p`; it aborts on a `NULL` array of arguments with `args_len` other than 0, or a
    /// `NULL` argument within it.
    ///
    /// # Safety
    ///
    /// `args` addresses `args_len` arguments, each a NUL-terminated string, unless `args_len` is
    /// 0: the array and the strings live, the strings unchanged, for `'p`, and nothing but the
    /// parse given them reaches the array meanwhile.
    unsafe fn adopt_args<'p>(args: *mut *mut c_char, args_len: usize) -> &'p mut [Arg<'p>] {
        if args_len == 0 {
            return &mut [];
        }
        if args.is_null() {
            std::process::abort();
        }
        // SAFETY: the caller's contract: `args` addresses `args_len` readable elements.
        let elements = unsafe { slice::from_raw_parts(args.cast_const(), args_len) };
        if elements.iter().any(|arg| arg.is_null()) {
            std::process::abort();
        }
        // SAFETY: the `args_len` elements are non-null, as just checked, and each a NUL-terminated
        // string that lives, unchanged, for `'p`, which only the parse given them reaches
        // meanwhile, by the caller's contract: valid `Arg`s, which are `#[repr(transparent)]` over
        // a non-null `char *`, borrowed exclusively for as long.
        unsafe { slice::from_raw_parts_mut(args.cast::<Arg<'p>>(), args_len) }
    }

    /// Writes `opt` to `*out` and returns its status, as C's parser fills the outcome its caller
    /// gives it and returns that outcome's status. It aborts on a `NULL` `out`.
    ///
    /// # Safety
    ///
    /// `out` is `NULL` or valid for a write of a `cli_opt`.
    unsafe fn write_outcome(out: *mut sys::cli_opt, opt: Opt<'_>) -> sys::cli_opt_status_t {
        if out.is_null() {
            std::process::abort();
        }
        let status = opt.as_ref().raw_status();
        // SAFETY: `out` is non-null and valid for a write of a `cli_opt`, the caller's contract;
        // `Opt` is `#[repr(transparent)]` over it, and the write reads nothing of the old value,
        // which needs no drop.
        unsafe { out.cast::<Opt<'_>>().write(opt) };
        status
    }

    /// `cli_opt_parser_init`: [`Parser::new`] over the table at `specs` and the `args_len`
    /// arguments at `args`, written to `*parser`, which it overwrites whole, as C's `memset`
    /// does. A panic aborts, as it cannot fail.
    ///
    /// The C stores `specs` and `args` unread, and the parse that follows reads them; here they
    /// are read at once, so a `NULL` parser, a `NULL` table, a `NULL` array of arguments with
    /// `args_len` other than 0, or a `NULL` argument within it aborts, where the C, built with
    /// `NDEBUG`, writes through the `NULL` parser or faults in that parse.
    ///
    /// # Safety
    ///
    /// `parser` is valid for a write of a `cli_opt_parser`; `specs` is a spec table, as
    /// [`SpecTable::from_ptr`] adopts one, and `args` addresses `args_len` arguments, each a
    /// NUL-terminated string, unless `args_len` is 0: all of them live, the strings unchanged,
    /// while C uses the parser, and nothing but that parse reaches the array meanwhile.
    #[unsafe(no_mangle)]
    unsafe extern "C" fn cli_opt_parser_init(
        parser: *mut sys::cli_opt_parser,
        specs: *const sys::cli_opt_spec,
        args: *mut *mut c_char,
        args_len: usize,
        flags: c_uint,
    ) {
        if parser.is_null() {
            std::process::abort();
        }
        // SAFETY: the caller's contract: `specs` is a table that lives, unchanged, while C uses
        // the parser, the lifetime this adoption takes.
        let Some(table) = (unsafe { SpecTable::from_ptr(specs) }) else {
            std::process::abort();
        };
        // SAFETY: the caller's contract: `args` addresses `args_len` arguments that live, the
        // strings unchanged, while C uses the parser, which only that parse reaches meanwhile.
        let args = unsafe { adopt_args(args, args_len) };
        let flags = ParseFlags::from_bits_retain(flags);
        let new = barrier(|| Parser::new(table, args, flags), || std::process::abort());
        // SAFETY: `parser` is non-null and valid for a write of a `cli_opt_parser`, the
        // caller's contract; `Parser` is `#[repr(transparent)]` over it, and the write reads
        // nothing of the old value, which needs no drop.
        unsafe { parser.cast::<Parser<'_>>().write(new) };
    }

    /// `cli_opt_parser_next`: [`ParserMut::parse_next`] of `*parser`, whose outcome it writes to
    /// `*opt`, overwriting it whole, as C's `memset` does, and whose status it returns. A `NULL`
    /// outcome or parser aborts, where the C, built with `NDEBUG`, writes through it; so does a
    /// panic, which no status the C returns describes.
    ///
    /// # Safety
    ///
    /// `opt` is valid for a write of a `cli_opt`, and `parser` is a parser `cli_opt_parser_init`
    /// set up and only this export has changed since: a valid `Parser`, as
    /// [`Parser`]'s adoption asks, whose table and arguments live, unchanged but for the order
    /// a parse gives the arguments, while C uses the parser and the outcomes it gives, and which
    /// nothing else reaches during the call.
    #[unsafe(no_mangle)]
    unsafe extern "C" fn cli_opt_parser_next(
        opt: *mut sys::cli_opt,
        parser: *mut sys::cli_opt_parser,
    ) -> sys::cli_opt_status_t {
        // SAFETY: the caller's contract: `parser` is a valid parser that nothing else reaches
        // during the call, whose table and arguments live while C uses it and its outcomes, the
        // lifetime this adoption takes.
        let Some(mut parser) = (unsafe { ParserMut::from_ptr(parser) }) else {
            std::process::abort();
        };
        let next = barrier(|| parser.parse_next(), || std::process::abort());
        // SAFETY: the caller's contract: `opt` is valid for a write of a `cli_opt`.
        unsafe { write_outcome(opt, next) }
    }

    /// `cli_opt_parse`: [`parse`] of the `args_len` arguments at `args` against the table at
    /// `specs`, whose outcome, the one that failed or the last, it writes to `*opt`, overwriting it
    /// whole, and whose status it returns: `CLI_OPT_STATUS_DONE` once every argument is parsed and
    /// every requirement met. A `NULL` outcome, table or array of arguments, or a `NULL` argument
    /// within it, aborts, where the C, built with `NDEBUG`, writes or reads through it; so does a
    /// panic, which no status the C returns describes.
    ///
    /// # Safety
    ///
    /// `opt` is valid for a write of a `cli_opt`; `specs` is a spec table, as
    /// [`SpecTable::from_ptr`] adopts one, and `args` addresses `args_len` arguments, each a
    /// NUL-terminated string, unless `args_len` is 0: all of them live, the strings unchanged,
    /// while C uses the outcome and the specs' variables, and nothing but this parse reaches the
    /// array during the call.
    #[unsafe(no_mangle)]
    unsafe extern "C" fn cli_opt_parse(
        opt: *mut sys::cli_opt,
        specs: *const sys::cli_opt_spec,
        args: *mut *mut c_char,
        args_len: usize,
        flags: c_uint,
    ) -> sys::cli_opt_status_t {
        if opt.is_null() {
            std::process::abort();
        }
        // SAFETY: the caller's contract: `specs` is a table that lives, unchanged, while C uses
        // the outcome and the variables, the lifetime this adoption takes.
        let Some(table) = (unsafe { SpecTable::from_ptr(specs) }) else {
            std::process::abort();
        };
        // SAFETY: the caller's contract: `args` addresses `args_len` arguments that live, the
        // strings unchanged, as long, which only this parse reaches during the call.
        let args = unsafe { adopt_args(args, args_len) };
        let flags = ParseFlags::from_bits_retain(flags);
        let outcome = barrier(
            || match parse(table, args, flags) {
                Ok((done, _)) => done,
                Err(failed) => failed,
            },
            || std::process::abort(),
        );
        // SAFETY: the caller's contract: `opt`, non-null, is valid for a write of a `cli_opt`.
        unsafe { write_outcome(opt, outcome) }
    }

    /// `cli_opt_foreach`: [`for_each`] of the `args_len` arguments at `args` against the table at
    /// `specs`, calling `callback` with each outcome and `callback_data`: the first value other
    /// than 0 it returns, or 0 once every argument is parsed. A panic is reported as -1, a failure
    /// `common.c`, the CLI's caller, reads as any other. A `NULL` table, callback or array of
    /// arguments, or a `NULL` argument within it, aborts, where the C, built with `NDEBUG`, reads
    /// or calls through it.
    ///
    /// # Safety
    ///
    /// `specs` is a spec table, as [`SpecTable::from_ptr`] adopts one, and `args` addresses
    /// `args_len` arguments, each a NUL-terminated string, unless `args_len` is 0: all of them
    /// live, the strings unchanged, while C uses the specs' variables, and nothing but this parse
    /// reaches the array during the call. `callback` may be called with `callback_data` as its
    /// second argument, any number of times, on the calling thread, during the call, with an
    /// outcome it only reads and whose address it does not keep once it returns, as
    /// [`ForeachCallbackFn::from_c`] asks.
    #[unsafe(no_mangle)]
    unsafe extern "C" fn cli_opt_foreach(
        specs: *const sys::cli_opt_spec,
        args: *mut *mut c_char,
        args_len: usize,
        flags: c_uint,
        callback: Option<unsafe extern "C" fn(*mut sys::cli_opt, *mut c_void) -> c_int>,
        callback_data: *mut c_void,
    ) -> c_int {
        // SAFETY: the caller's contract: `specs` is a table that lives, unchanged, while C uses
        // the variables, the lifetime this adoption takes.
        let Some(table) = (unsafe { SpecTable::from_ptr(specs) }) else {
            std::process::abort();
        };
        let Some(callback) = callback else {
            std::process::abort();
        };
        // SAFETY: the caller's contract: `args` addresses `args_len` arguments that live, the
        // strings unchanged, as long, which only this parse reaches during the call.
        let args = unsafe { adopt_args(args, args_len) };
        let flags = ParseFlags::from_bits_retain(flags);
        // SAFETY: the caller's contract: `callback` may be called with `callback_data`, on this
        // thread, during this call, which the callable does not outlive, with outcomes it only
        // reads and does not keep.
        let callback = unsafe { ForeachCallbackFn::from_c(callback, callback_data) };
        barrier(
            || match for_each(table, args, flags, callback) {
                ControlFlow::Continue(()) => 0,
                ControlFlow::Break(code) => code.get(),
            },
            || -1,
        )
    }

    /// `cli_opt_status_fprint`: [`OptRef::write_status`] of `*opt` to the stream `file`, after
    /// `command` when it is not `NULL`, read from the table that starts at the outcome's spec,
    /// which holds that spec's later choices. Returns 0 once it is written, and -1 when a write
    /// fails, as `opt.h` documents, `errno` holding the stream's error, or `EINVAL` for an
    /// outcome the message cannot name, a missing value or argument with no spec, whose spec the
    /// C reads through a null pointer. A panic is reported as a failure.
    ///
    /// The C returns the count of its last piece's `fprintf` on success: no caller reads it. The
    /// `command`, the argument and the names are written as their bytes, through [`FileWriter`],
    /// each piece in one `fwrite`, which a failure, `EINTR` included, ends unretried, as it ends
    /// C's `fprintf`.
    ///
    /// # Safety
    ///
    /// `file` is an open stream that stays open during the call, every other access to it
    /// holding its lock, as C's stdio routines do; `command` is `NULL` or a NUL-terminated
    /// string; and `opt` is an outcome C's parser wrote, or a zeroed one: a valid `Opt`, as
    /// [`Opt`](super::Opt)'s adoption asks, whose spec, when it has one, is an element of a spec
    /// table [`SpecTable::from_ptr`] adopts from it. All of them live, unchanged, for the call.
    #[unsafe(no_mangle)]
    unsafe extern "C" fn cli_opt_status_fprint(
        file: *mut sys::FILE,
        command: *const c_char,
        opt: *const sys::cli_opt,
    ) -> c_int {
        // SAFETY: the caller's contract: `file` is an open stream for the call, every other
        // access holding its lock, as `FileRef::from_ptr` asks of a stream.
        let Some(file) = (unsafe { FileRef::from_ptr(file) }) else {
            set_errno(Errno::EINVAL as c_int);
            return -1;
        };
        // SAFETY: the caller's contract: a non-null `command` is a NUL-terminated string that
        // lives, unchanged, for the call.
        let command = (!command.is_null()).then(|| unsafe { CStr::from_ptr(command) });
        // SAFETY: the caller's contract: `opt` is a valid outcome that lives, unchanged, for
        // the call, the lifetime this adoption takes.
        let Some(opt) = (unsafe { OptRef::from_ptr(opt.cast_mut()) }) else {
            set_errno(Errno::EINVAL as c_int);
            return -1;
        };
        let end = [Spec::zeroed()];
        let specs = opt
            .spec()
            // SAFETY: the caller's contract: the outcome's spec is an element of a table
            // ended by a spec of kind `CLI_OPT_TYPE_NONE`, live and unchanged for the call.
            .and_then(|spec| unsafe { SpecTable::from_ptr(spec.as_ptr()) })
            .or_else(|| SpecTable::new(&end));
        let Some(specs) = specs else {
            set_errno(Errno::EINVAL as c_int);
            return -1;
        };
        let command = command.map(CStr::to_bytes);
        let written = barrier(
            || opt.write_status_bytes(&mut FileWriter(file), command, specs),
            || Err(std::io::Error::other("panicked")),
        );
        match written {
            Ok(()) => 0,
            Err(e) => {
                // The stream's failures leave `errno` as `fprintf` set it; the refusal of an
                // outcome without a spec sets none of its own.
                if e.kind() == std::io::ErrorKind::InvalidInput && e.raw_os_error().is_none() {
                    set_errno(Errno::EINVAL as c_int);
                }
                -1
            }
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

/// The parser's internal routines against their contract, over tables built through the safe
/// API: which spec each argument selects, how a choice is stepped over and which requirement is
/// missing, with the C defects they correct. No C runs.
#[cfg(test)]
mod unit_priv {
    use core::cell::Cell;
    use core::ffi::CStr;

    use super::{
        Arg, LongForm, ParseFlags, Parser, Spec, SpecRef, SpecTable, Target, Usage, check_required,
        find_arg, find_for_sort, find_long, find_short, is_choice, is_given, reorder_gnu_style,
        skip_choice_args, write_spec_name,
    };

    /// A parser over `table` with no arguments, through which the lookups read the table.
    fn parser(table: SpecTable<'_>) -> Parser<'_> {
        Parser::new(table, &mut [], ParseFlags::DEFAULT)
    }

    /// The position of a found spec in `table`.
    fn at<'p, T>(table: SpecTable<'p>, found: Option<(SpecRef<'p, 'p>, T)>) -> Option<(usize, T)> {
        found.map(|(spec, rest)| (table.position(spec).expect("in the table"), rest))
    }

    /// Each kind of long argument selects the first spec that takes it, by name, `no-` and
    /// `=value`, and the bare `--` the literal.
    #[test]
    fn long_arguments_select_their_specs() {
        let flag = Cell::new(0);
        let specs = [
            Spec::new(Target::Bool(Some(&flag))).with_name(c"debug"),
            Spec::new(Target::Switch(None)).with_name(c"read"),
            Spec::new(Target::Accumulator(None)).with_name(c"verbose"),
            Spec::new(Target::Value(None)).with_name(c"output"),
            Spec::new(Target::Literal),
            Spec::new(Target::Value(None)).with_name(c"no-read"),
            Spec::new(Target::Arg(None)).with_name(c"file"),
            Spec::default(),
        ];
        let table = SpecTable::new(&specs).unwrap();
        let p = parser(table);
        let find = |arg: &'static CStr| at(table, find_long(p.as_ref(), arg));
        assert_eq!(find(c"debug"), Some((0, LongForm::Plain)));
        assert_eq!(find(c"no-debug"), Some((0, LongForm::Negated)));
        assert_eq!(find(c"read"), Some((1, LongForm::Plain)));
        // A switch has no `no-` form: the value spec of that name is the one.
        assert_eq!(find(c"no-read"), Some((5, LongForm::Plain)));
        // An accumulator is selected by its alias alone, an argument never by a name.
        assert_eq!(find(c"verbose"), None);
        assert_eq!(find(c"file"), None);
        assert_eq!(find(c"output"), Some((3, LongForm::Plain)));
        assert_eq!(
            find(c"output=out.txt"),
            Some((3, LongForm::Valued(Some(c"out.txt"))))
        );
        assert_eq!(
            find(c"output=a=b"),
            Some((3, LongForm::Valued(Some(c"a=b"))))
        );
        assert_eq!(find(c"output="), Some((3, LongForm::Valued(None))));
        assert_eq!(find(c""), Some((4, LongForm::Plain)));
        assert_eq!(find(c"outputs=x"), None);
        assert_eq!(find(c"out=x"), None);
        assert_eq!(find(c"debug=1"), None);
        assert_eq!(find(c"unknown"), None);
        assert_eq!(flag.get(), 0, "a lookup writes nothing");
    }

    /// The first matching spec wins, whatever comes after it.
    #[test]
    fn the_first_spec_that_matches_is_the_one() {
        let specs = [
            Spec::new(Target::Switch(None)).with_name(c"x"),
            Spec::new(Target::Bool(None)).with_name(c"x"),
            Spec::new(Target::Literal),
            Spec::new(Target::Literal),
            Spec::default(),
        ];
        let table = SpecTable::new(&specs).unwrap();
        let p = parser(table);
        assert_eq!(
            at(table, find_long(p.as_ref(), c"x")),
            Some((0, LongForm::Plain))
        );
        assert_eq!(
            at(table, find_long(p.as_ref(), c"no-x")),
            Some((1, LongForm::Negated))
        );
        assert_eq!(
            at(table, find_long(p.as_ref(), c"")),
            Some((2, LongForm::Plain))
        );
    }

    /// A bool spec without a name matches no `no-` argument, where the C compares the argument
    /// with its `NULL` name.
    #[test]
    fn a_nameless_bool_has_no_negation() {
        let specs = [
            Spec::new(Target::Bool(None)).with_alias(b'd'),
            Spec::new(Target::Value(None)).with_alias(b'o'),
            Spec::default(),
        ];
        let table = SpecTable::new(&specs).unwrap();
        let p = parser(table);
        assert!(find_long(p.as_ref(), c"no-").is_none());
        assert!(find_long(p.as_ref(), c"no-d").is_none());
        assert!(find_long(p.as_ref(), c"=x").is_none());
    }

    /// A parser without a table finds nothing.
    #[test]
    fn a_parser_without_a_table_finds_nothing() {
        let mut p = Parser::default();
        assert!(find_long(p.as_ref(), c"").is_none());
        assert!(find_short(p.as_ref(), c"x").is_none());
        assert!(find_arg(&mut p.as_mut()).is_none());
    }

    /// A short option selects the first spec of its alias, and a value spec takes the rest of
    /// the argument as its value.
    #[test]
    fn short_options_select_their_specs() {
        let specs = [
            Spec::new(Target::Value(None)).with_alias(b'o'),
            Spec::new(Target::Bool(None)).with_alias(b'd'),
            Spec::new(Target::Accumulator(None)).with_alias(b'v'),
            Spec::new(Target::Arg(None)).with_alias(b'a'),
            Spec::new(Target::Switch(None)).with_alias(b'd'),
            Spec::new(Target::Switch(None)).with_alias(0xE9),
            Spec::default(),
        ];
        let table = SpecTable::new(&specs).unwrap();
        let p = parser(table);
        let find = |arg: &'static CStr| at(table, find_short(p.as_ref(), arg));
        assert_eq!(find(c"ofile"), Some((0, Some(c"file"))));
        assert_eq!(find(c"o"), Some((0, None)));
        assert_eq!(find(c"o-"), Some((0, Some(c"-"))));
        assert_eq!(find(c"d"), Some((1, None)));
        // The rest of a bundle is no value of a spec that takes none.
        assert_eq!(find(c"dvv"), Some((1, None)));
        assert_eq!(find(c"vd"), Some((2, None)));
        assert_eq!(find(c"a"), Some((3, None)));
        assert_eq!(find(c"\xe9"), Some((5, None)));
        assert_eq!(find(c"x"), None);
    }

    /// The bare `-` selects no spec, where the C selects the first spec without an alias and
    /// reads past the argument for a value spec; and a spec without an alias is never selected.
    #[test]
    fn no_short_option_selects_a_spec_without_an_alias() {
        let specs = [
            Spec::new(Target::Value(None)).with_name(c"config-env"),
            Spec::new(Target::Switch(None)).with_name(c"help"),
            Spec::new(Target::Bool(None)).with_alias(b'q'),
            Spec::default(),
        ];
        let table = SpecTable::new(&specs).unwrap();
        let p = parser(table);
        assert!(find_short(p.as_ref(), c"").is_none());
        assert_eq!(at(table, find_short(p.as_ref(), c"q")), Some((2, None)));
    }

    /// Bare arguments take the argument specs in order, each once, and the remaining arguments
    /// spec every one after them, without counting.
    #[test]
    fn bare_arguments_take_the_argument_specs_in_order() {
        let specs = [
            Spec::new(Target::Arg(None)),
            Spec::new(Target::Bool(None)).with_alias(b'd'),
            Spec::new(Target::Arg(None)),
            Spec::new(Target::Args(None)),
            Spec::new(Target::Arg(None)),
            Spec::default(),
        ];
        let table = SpecTable::new(&specs).unwrap();
        let mut p = parser(table);
        let mut next = || find_arg(&mut p.as_mut()).and_then(|s| table.position(s));
        assert_eq!(next(), Some(0));
        assert_eq!(next(), Some(2));
        assert_eq!(next(), Some(3));
        assert_eq!(next(), Some(3));
        assert_eq!(p.as_ref().arg_idx(), 2);

        let specs = [Spec::new(Target::Arg(None)), Spec::default()];
        let table = SpecTable::new(&specs).unwrap();
        let mut p = parser(table);
        assert!(find_arg(&mut p.as_mut()).is_some());
        assert!(find_arg(&mut p.as_mut()).is_none());
        assert_eq!(p.as_ref().arg_idx(), 1);
        p.as_mut().set_arg_idx(usize::MAX);
        assert!(find_arg(&mut p.as_mut()).is_none());
    }

    /// A choice: [`is_choice`] holds at each spec followed by one marked [`Usage::CHOICE`],
    /// not at the last spec nor past it.
    #[test]
    fn a_spec_followed_by_a_choice_is_a_choice() {
        let specs = [
            Spec::new(Target::Switch(None)),
            Spec::new(Target::Switch(None)).with_usage(Usage::CHOICE),
            Spec::new(Target::Arg(None)).with_usage(Usage::CHOICE | Usage::REQUIRED),
            Spec::new(Target::Switch(None)),
            Spec::new(Target::Switch(None)).with_usage(Usage::CHOICE),
            Spec::default(),
        ];
        let table = SpecTable::new(&specs).unwrap();
        let choices: Vec<_> = (0..7).map(|i| is_choice(table, i)).collect();
        assert_eq!(choices, [true, true, false, true, false, false, false]);
        assert!(!is_choice(table, usize::MAX));
    }

    /// An option of a choice counts its argument specs as matched, from any of its specs, and
    /// a spec outside every choice, or outside the table, counts none.
    #[test]
    fn a_choice_given_counts_its_arguments() {
        let specs = [
            Spec::new(Target::Switch(None)).with_alias(b'a'),
            Spec::new(Target::Arg(None)).with_usage(Usage::CHOICE),
            Spec::new(Target::Switch(None))
                .with_alias(b'b')
                .with_usage(Usage::CHOICE),
            Spec::new(Target::Arg(None)).with_usage(Usage::CHOICE),
            Spec::new(Target::Switch(None)).with_alias(b'c'),
            Spec::new(Target::Arg(None)),
            Spec::default(),
        ];
        let table = SpecTable::new(&specs).unwrap();
        let mut p = parser(table);
        for (index, after) in [(0, 2), (2, 4), (3, 6), (4, 6), (5, 6)] {
            skip_choice_args(table.get(index).unwrap(), &mut p.as_mut());
            assert_eq!(p.as_ref().arg_idx(), after, "{index}");
        }
        let other = [Spec::new(Target::Arg(None)), Spec::default()];
        let other = SpecTable::new(&other).unwrap();
        skip_choice_args(other.get(0).unwrap(), &mut p.as_mut());
        assert_eq!(p.as_ref().arg_idx(), 6);
        skip_choice_args(table.get(0).unwrap(), &mut Parser::default().as_mut());
    }

    /// A table whose first spec is marked a choice starts its choice there, where the C steps
    /// back before the table's start.
    #[test]
    fn a_choice_starts_no_earlier_than_the_table() {
        let specs = [
            Spec::new(Target::Arg(None)).with_usage(Usage::CHOICE),
            Spec::new(Target::Arg(None)).with_usage(Usage::CHOICE),
            Spec::new(Target::Switch(None)),
            Spec::default(),
        ];
        let table = SpecTable::new(&specs).unwrap();
        let mut p = parser(table);
        skip_choice_args(table.get(1).unwrap(), &mut p.as_mut());
        assert_eq!(p.as_ref().arg_idx(), 2);
        let specs = [
            Spec::new(Target::Arg(None)).with_usage(Usage::CHOICE),
            Spec::new(Target::Switch(None)),
            Spec::default(),
        ];
        let table = SpecTable::new(&specs).unwrap();
        let mut p = parser(table);
        skip_choice_args(table.get(0).unwrap(), &mut p.as_mut());
        assert_eq!(p.as_ref().arg_idx(), 0, "a choice of one is none");
    }

    /// The specs given are compared by address: an equal spec of another table is not given.
    #[test]
    fn given_specs_are_found_by_address() {
        let specs = [
            Spec::new(Target::Literal),
            Spec::new(Target::Literal),
            Spec::default(),
        ];
        let table = SpecTable::new(&specs).unwrap();
        let (a, b) = (table.get(0).unwrap(), table.get(1).unwrap());
        assert!(is_given(&[b, a], a));
        assert!(!is_given(&[b], a));
        assert!(!is_given(&[], a));
    }

    /// The first requirement not given is reported by its spec, the first of its choice, and
    /// one option of a required choice meets it.
    #[test]
    fn requirements_are_met_by_a_spec_or_one_choice() {
        let specs = [
            Spec::new(Target::Switch(None)).with_alias(b'x'),
            Spec::new(Target::Arg(None)).with_usage(Usage::REQUIRED),
            Spec::new(Target::Switch(None))
                .with_alias(b'a')
                .with_usage(Usage::REQUIRED),
            Spec::new(Target::Switch(None))
                .with_alias(b'b')
                .with_usage(Usage::CHOICE),
            Spec::new(Target::Arg(None)).with_usage(Usage::CHOICE),
            Spec::new(Target::Arg(None)).with_usage(Usage::REQUIRED),
            Spec::default(),
        ];
        let table = SpecTable::new(&specs).unwrap();
        let spec = |i| table.get(i).unwrap();
        let missing = |given: &[usize]| {
            let given: Vec<_> = given.iter().map(|&i| spec(i)).collect();
            check_required(table, &given)
                .err()
                .and_then(|s| table.position(s))
        };
        assert_eq!(missing(&[]), Some(1));
        assert_eq!(missing(&[1]), Some(2));
        assert_eq!(missing(&[1, 0]), Some(2));
        for choice in [2, 3, 4] {
            assert_eq!(missing(&[1, choice]), Some(5), "{choice}");
            assert_eq!(missing(&[5, choice, 1]), None, "{choice}");
        }
        assert_eq!(missing(&[5, 1]), Some(2));
        let none = [Spec::new(Target::Arg(None)), Spec::default()];
        assert!(check_required(SpecTable::new(&none).unwrap(), &[]).is_ok());
    }

    /// A requirement on a later choice holds for the rest of its choice.
    #[test]
    fn a_requirement_inside_a_choice_holds_for_its_rest() {
        let specs = [
            Spec::new(Target::Switch(None)).with_alias(b'a'),
            Spec::new(Target::Switch(None))
                .with_alias(b'b')
                .with_usage(Usage::CHOICE | Usage::REQUIRED),
            Spec::new(Target::Switch(None))
                .with_alias(b'c')
                .with_usage(Usage::CHOICE),
            Spec::default(),
        ];
        let table = SpecTable::new(&specs).unwrap();
        let spec = |i| table.get(i).unwrap();
        assert_eq!(
            check_required(table, &[])
                .err()
                .and_then(|s| table.position(s)),
            Some(1)
        );
        assert!(check_required(table, &[spec(2)]).is_ok());
        assert!(check_required(table, &[spec(0)]).is_err());
    }

    /// The spec a GNU-style reordering selects for each kind of argument, and whether it still
    /// needs a value: a long option unless it gave one, a bundle by its last option or the first
    /// that takes the rest as its value; nothing for a bare argument, an unknown option, a bundle
    /// holding one, or the bare `-`.
    #[test]
    fn reordering_selects_options_as_parsing_does() {
        let specs = [
            Spec::new(Target::Bool(None))
                .with_name(c"debug")
                .with_alias(b'd'),
            Spec::new(Target::Value(None))
                .with_name(c"output")
                .with_alias(b'o'),
            Spec::new(Target::Literal),
            Spec::new(Target::Switch(None)).with_name(c"help"),
            Spec::default(),
        ];
        let table = SpecTable::new(&specs).unwrap();
        let p = parser(table);
        let find = |arg: &'static CStr| {
            find_for_sort(p.as_ref(), arg).map(|(s, needs)| (table.position(s).unwrap(), needs))
        };
        assert_eq!(find(c"--debug"), Some((0, true)));
        assert_eq!(find(c"--no-debug"), Some((0, true)));
        assert_eq!(find(c"--output"), Some((1, true)));
        assert_eq!(find(c"--output=x"), Some((1, false)));
        assert_eq!(find(c"--output="), Some((1, false)));
        assert_eq!(find(c"--"), Some((2, true)));
        assert_eq!(find(c"-d"), Some((0, true)));
        assert_eq!(find(c"-dd"), Some((0, true)));
        assert_eq!(find(c"-do"), Some((1, true)));
        assert_eq!(find(c"-dofile"), Some((1, false)));
        assert_eq!(find(c"-ofile"), Some((1, false)));
        for none in [c"file", c"", c"-", c"-x", c"-dx", c"--nope", c"-h"] {
            assert_eq!(find(none), None, "{none:?}");
        }
    }

    /// A reordering moves each option, with the value it takes, ahead of the bare arguments
    /// before it, keeping both orders, and says so; one with nothing to move says not.
    #[test]
    fn reordering_moves_options_and_values_forward() {
        let specs = [
            Spec::new(Target::Bool(None)).with_alias(b'd'),
            Spec::new(Target::Value(None)).with_alias(b'o'),
            Spec::default(),
        ];
        let table = SpecTable::new(&specs).unwrap();
        let mut args = [c"x", c"a", c"-d", c"b", c"-o", c"v", c"c", c"-ow"].map(Arg::new);
        let mut p = Parser::new(table, &mut args, ParseFlags::FORCE_GNU);
        p.as_mut().set_idx(1);
        assert!(reorder_gnu_style(&mut p.as_mut()));
        assert!(!p.as_ref().needs_sort());
        assert_eq!(p.as_ref().idx(), 1);
        let order: Vec<_> = p.as_ref().args().iter().map(|a| a.as_c_str()).collect();
        assert_eq!(order, [c"x", c"-d", c"-o", c"v", c"-ow", c"a", c"b", c"c"]);
        p.as_mut().set_idx(5);
        assert!(
            !reorder_gnu_style(&mut p.as_mut()),
            "no option after the bare arguments"
        );
        let mut args = [c"a", c"b"].map(Arg::new);
        let mut p = Parser::new(table, &mut args, ParseFlags::FORCE_GNU);
        assert!(!reorder_gnu_style(&mut p.as_mut()));
        assert!(!p.as_ref().needs_sort());
    }

    /// A reordering stops at a literal `--`, moves the parser to a last option missing a
    /// required value, leaves one whose value is optional in place, moves nothing from the
    /// parser's first argument, and nothing once the parser is literal, clearing its need to
    /// reorder each time.
    #[test]
    fn reordering_stops_where_parsing_must_not_move() {
        let reorder = |line: &[&'static CStr], idx: usize, literal: bool| {
            let mut args: Vec<_> = line.iter().copied().map(Arg::new).collect();
            let specs = [
                Spec::new(Target::Bool(None)).with_alias(b'd'),
                Spec::new(Target::Value(None)).with_alias(b'o'),
                Spec::new(Target::Value(None))
                    .with_alias(b'S')
                    .with_usage(Usage::VALUE_OPTIONAL),
                Spec::new(Target::Literal),
                Spec::default(),
            ];
            let table = SpecTable::new(&specs).unwrap();
            let mut p = Parser::new(table, &mut args, ParseFlags::FORCE_GNU);
            p.as_mut().set_idx(idx);
            p.as_mut().set_in_literal(literal);
            let changed = reorder_gnu_style(&mut p.as_mut());
            assert!(!p.as_ref().needs_sort());
            let order: Vec<_> = p
                .as_ref()
                .args()
                .iter()
                .map(|a| a.as_c_str().to_owned())
                .collect();
            (changed, p.as_ref().idx(), order)
        };
        let owned = |line: &[&CStr]| line.iter().map(|a| (*a).to_owned()).collect::<Vec<_>>();
        assert_eq!(
            reorder(&[c"a", c"--", c"-d"], 0, false),
            (false, 0, owned(&[c"a", c"--", c"-d"]))
        );
        assert_eq!(
            reorder(&[c"a", c"-d", c"b", c"-o"], 0, false),
            (true, 3, owned(&[c"-d", c"a", c"b", c"-o"]))
        );
        assert_eq!(
            reorder(&[c"a", c"-d", c"b", c"-S"], 0, false),
            (true, 0, owned(&[c"-d", c"a", c"b", c"-S"]))
        );
        assert_eq!(
            reorder(&[c"-d", c"a"], 0, false),
            (false, 0, owned(&[c"-d", c"a"]))
        );
        assert_eq!(
            reorder(&[c"--", c"a", c"-d"], 1, true),
            (false, 1, owned(&[c"--", c"a", c"-d"]))
        );
    }

    /// The name of each kind of spec, in one write, a missing string written `(null)`.
    #[test]
    fn spec_names_are_written_whole() {
        let specs = [
            Spec::new(Target::Arg(None))
                .with_value_name(c"<file>")
                .with_alias(b'f'),
            Spec::new(Target::Args(None)),
            Spec::new(Target::Switch(None))
                .with_alias(b'f')
                .with_name(c"force"),
            Spec::new(Target::Switch(None))
                .with_alias(b'f')
                .with_name(c"force")
                .with_usage(Usage::SHOW_LONG),
            Spec::new(Target::Value(None)).with_name(c"output"),
            Spec::new(Target::Bool(None)),
            Spec::new(Target::Switch(None)).with_alias(0xE9),
            Spec::default(),
        ];
        let table = SpecTable::new(&specs).unwrap();
        let expected: [&[u8]; 7] = [
            b"<file>",
            b"(null)",
            b"-f",
            b"--force",
            b"--output",
            b"--(null)",
            b"-\xe9",
        ];
        for (spec, expected) in table.iter().zip(expected) {
            let mut out = Pieces::default();
            write_spec_name(&mut out, spec).unwrap();
            assert_eq!(out.0, [expected], "{spec:?}");
        }
    }

    /// Each write the writer is given, kept apart.
    #[derive(Default)]
    struct Pieces(Vec<Vec<u8>>);

    impl std::io::Write for Pieces {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.push(buf.to_vec());
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    /// An argument lent by a parse is a borrow of its array, for `'p`.
    #[test]
    fn values_lent_by_a_lookup_outlive_the_parser() {
        let specs = [
            Spec::new(Target::Value(None)).with_alias(b'o'),
            Spec::default(),
        ];
        let table = SpecTable::new(&specs).unwrap();
        let args = [Arg::new(c"-ofile")];
        let value = {
            let p = parser(table);
            let arg = args[0].as_c_str();
            let rest = CStr::from_bytes_until_nul(&arg.to_bytes_with_nul()[1..]).unwrap();
            find_short(p.as_ref(), rest).and_then(|(_, v)| v)
        };
        assert_eq!(value, Some(c"file"));
    }
}
