//! Interfaces and types to be used as symbols for the
//! [`StringInterner`](`crate::StringInterner`).
//!
//! The [`StringInterner::get_or_intern`](`crate::StringInterner::get_or_intern`)
//! method returns `Symbol` types that allow to look-up the original string
//! using [`StringInterner::resolve`](`crate::StringInterner::resolve`).

use core::num::{
    NonZeroU16,
    NonZeroU32,
    NonZeroUsize,
};

/// Types implementing this trait can be used as symbols for string interners.
///
/// The [`StringInterner::get_or_intern`](`crate::StringInterner::get_or_intern`)
/// method returns `Symbol` types that allow to look-up the original string
/// using [`StringInterner::resolve`](`crate::StringInterner::resolve`).
///
/// # Note
///
/// Optimal symbols allow for efficient comparisons and have a small memory footprint.
pub trait Symbol: Copy + Eq {
    /// Creates a symbol from a `usize`.
    ///
    /// Returns `None` if `index` is out of bounds for the symbol.
    fn try_from_usize(index: usize) -> Option<Self>;

    /// Returns the `usize` representation of `self`.
    fn to_usize(self) -> usize;
}

/// Creates the symbol `S` from the given `usize`.
///
/// # Panics
///
/// Panics if the conversion is invalid.
pub(crate) fn expect_valid_symbol<S>(index: usize) -> S
where
    S: Symbol,
{
    S::try_from_usize(index).expect("encountered invalid symbol")
}

/// The symbol type that is used by default.
pub type DefaultSymbol = SymbolU32;

macro_rules! gen_symbol_for {
    (
        $( #[$doc:meta] )*
        struct $name:ident($non_zero:ty; $base_ty:ty);
    ) => {
        $( #[$doc] )*
        #[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name {
            value: $non_zero,
        }

        impl Symbol for $name {
            #[inline]
            fn try_from_usize(index: usize) -> Option<Self> {
                if index < usize::MAX {
                    return Some(Self {
                        value: unsafe { <$non_zero>::new_unchecked(index as $base_ty + 1) },
                    })
                }
                None
            }

            #[inline]
            fn to_usize(self) -> usize {
                self.value.get() as usize - 1
            }
        }
    };
}
gen_symbol_for!(
    /// Symbol that is 16-bit in size.
    ///
    /// Is space-optimized for used in `Option`.
    struct SymbolU16(NonZeroU16; u16);
);
gen_symbol_for!(
    /// Symbol that is 32-bit in size.
    ///
    /// Is space-optimized for used in `Option`.
    struct SymbolU32(NonZeroU32; u32);
);
gen_symbol_for!(
    /// Symbol that is the same size as a pointer (`usize`).
    ///
    /// Is space-optimized for used in `Option`.
    struct SymbolUsize(NonZeroUsize; usize);
);

#[cfg(test)]
mod tests {
    use super::*;
    use core::mem::size_of;

    #[test]
    fn same_size_as_u32() {
        assert_eq!(size_of::<DefaultSymbol>(), size_of::<u32>());
    }

    #[test]
    fn same_size_as_optional() {
        assert_eq!(
            size_of::<DefaultSymbol>(),
            size_of::<Option<DefaultSymbol>>()
        );
    }
}
