//! The one flags style of the safe API: `Copy` newtypes over the bits, with
//! named constants, set operations and `|`/`&`/`-`/`!` (I-8).

/// Define a flags type: `bitflags! { /// doc  pub struct Name: u32 { /// doc
/// A = 1; … } }`. Unknown bits are refused by `from_bits` and dropped by
/// `from_bits_truncate`, so a value only ever holds known flags.
macro_rules! bitflags {
    ($(#[$m:meta])* pub struct $name:ident: $t:ty { $($(#[$fm:meta])* $f:ident = $v:expr;)* }) => {
        $(#[$m])*
        #[derive(Clone, Copy, Default, PartialEq, Eq, Hash)]
        pub struct $name($t);

        #[allow(dead_code)]
        impl $name {
            $($(#[$fm])* pub const $f: $name = $name($v);)*

            /// No flag.
            pub const fn empty() -> $name {
                $name(0)
            }
            /// Every flag.
            pub const fn all() -> $name {
                $name(0 $(| $v)*)
            }
            /// The bits.
            pub const fn bits(self) -> $t {
                self.0
            }
            /// The flags of `bits`, `None` if any bit is not a flag.
            pub const fn from_bits(bits: $t) -> Option<$name> {
                if bits & !Self::all().0 == 0 { Some($name(bits)) } else { None }
            }
            /// The flags of `bits`, ignoring the bits that are not flags.
            pub const fn from_bits_truncate(bits: $t) -> $name {
                $name(bits & Self::all().0)
            }
            /// Whether no flag is set.
            pub const fn is_empty(self) -> bool {
                self.0 == 0
            }
            /// Whether every flag of `other` is set.
            pub const fn contains(self, other: $name) -> bool {
                self.0 & other.0 == other.0
            }
            /// Whether any flag of `other` is set.
            pub const fn intersects(self, other: $name) -> bool {
                self.0 & other.0 != 0
            }
            /// Set the flags of `other`.
            pub fn insert(&mut self, other: $name) {
                self.0 |= other.0;
            }
            /// Clear the flags of `other`.
            pub fn remove(&mut self, other: $name) {
                self.0 &= !other.0;
            }
            /// Set or clear the flags of `other`.
            pub fn set(&mut self, other: $name, on: bool) {
                if on { self.insert(other) } else { self.remove(other) }
            }
            /// The flags as C's bits (crate-internal name for the C side).
            pub(crate) const fn raw(self) -> $t {
                self.0
            }
        }

        impl core::ops::BitOr for $name {
            type Output = $name;
            fn bitor(self, o: $name) -> $name { $name(self.0 | o.0) }
        }
        impl core::ops::BitOrAssign for $name {
            fn bitor_assign(&mut self, o: $name) { self.0 |= o.0 }
        }
        impl core::ops::BitAnd for $name {
            type Output = $name;
            fn bitand(self, o: $name) -> $name { $name(self.0 & o.0) }
        }
        impl core::ops::BitAndAssign for $name {
            fn bitand_assign(&mut self, o: $name) { self.0 &= o.0 }
        }
        impl core::ops::Sub for $name {
            type Output = $name;
            fn sub(self, o: $name) -> $name { $name(self.0 & !o.0) }
        }
        impl core::ops::Not for $name {
            type Output = $name;
            fn not(self) -> $name { $name(!self.0 & Self::all().0) }
        }
        impl core::fmt::Debug for $name {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                let mut first = true;
                f.write_str(concat!(stringify!($name), "("))?;
                $(
                    if $v != 0 && self.0 & $v == $v {
                        if !first { f.write_str(" | ")?; }
                        first = false;
                        f.write_str(stringify!($f))?;
                    }
                )*
                let _ = &mut first;
                f.write_str(")")
            }
        }
    };
}

pub(crate) use bitflags;
