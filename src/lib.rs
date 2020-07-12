#![doc(html_root_url = "https://docs.rs/crate/string-interner/0.8.0")]
#![cfg_attr(not(feature = "std"), no_std)]
#![deny(missing_docs)]

//! Caches strings efficiently, with minimal memory footprint and associates them with unique symbols.
//! These symbols allow constant time comparisons and look-ups to the underlying interned strings.
//!
//! ### Example: Interning & Symbols
//!
//! ```
//! use string_interner::StringInterner;
//!
//! let mut interner = StringInterner::default();
//! let sym0 = interner.get_or_intern("Elephant");
//! let sym1 = interner.get_or_intern("Tiger");
//! let sym2 = interner.get_or_intern("Horse");
//! let sym3 = interner.get_or_intern("Tiger");
//! assert_ne!(sym0, sym1);
//! assert_ne!(sym0, sym2);
//! assert_ne!(sym1, sym2);
//! assert_eq!(sym1, sym3); // same!
//! ```
//!
//! ### Example: Creation by `FromIterator`
//!
//! ```
//! # use string_interner::DefaultStringInterner;
//! let interner = vec!["Elephant", "Tiger", "Horse", "Tiger"]
//! 	.into_iter()
//! 	.collect::<DefaultStringInterner>();
//! ```
//!
//! ### Example: Look-up
//!
//! ```
//! # use string_interner::StringInterner;
//! let mut interner = StringInterner::default();
//! let sym = interner.get_or_intern("Banana");
//! assert_eq!(interner.resolve(sym), Some("Banana"));
//! ```
//!
//! ### Example: Iteration
//!
//! ```
//! # use string_interner::DefaultStringInterner;
//! let interner = vec!["Earth", "Water", "Fire", "Air"]
//! 	.into_iter()
//! 	.collect::<DefaultStringInterner>();
//! for (sym, str) in interner {
//! 	// iteration code here!
//! }
//! ```

#[cfg(test)]
mod tests;

#[cfg(feature = "serde-1")]
mod serde_impl;

mod symbol;

pub use crate::symbol::{
    DefaultSymbol,
    Symbol,
};
use cfg_if::cfg_if;
use core::{
    hash::{
        BuildHasher,
        Hash,
        Hasher,
    },
    iter,
    iter::FromIterator,
    marker,
    pin::Pin,
    ptr::NonNull,
    slice,
};

cfg_if! {
    if #[cfg(feature = "std")] {
        use std::{
            collections::{
                hash_map::RandomState,
                HashMap,
            },
            vec,
        };
    } else {
        extern crate alloc;
        use alloc::{
            collections::{
                btree_map::BTreeMap,
            },
            vec,
        };
    }
}

/// Internal reference to an interned `str`.
///
/// This is a self-referential from the interners string map
/// into the interner's actual vector of strings.
#[derive(Debug, Copy, Clone, Eq)]
struct PinnedStr(NonNull<str>);

impl PinnedStr {
    /// Creates a new `PinnedStr` from the given `str`.
    fn from_str(val: &str) -> Self {
        PinnedStr(NonNull::from(val))
    }

    /// Creates a new `PinnedStr` from the given pinned `str`.
    fn from_pin(pinned: Pin<&str>) -> Self {
        PinnedStr(NonNull::from(&*pinned))
    }

    /// Returns a shared reference to the underlying `str`.
    fn as_str(&self) -> &str {
        // SAFETY: This is safe since we only ever operate on interned `str`
        //         that are never moved around in memory to avoid danling
        //         references.
        unsafe { self.0.as_ref() }
    }
}

impl Hash for PinnedStr {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.as_str().hash(state)
    }
}

impl PartialEq for PinnedStr {
    fn eq(&self, other: &Self) -> bool {
        self.as_str() == other.as_str()
    }
}

/// `StringInterner` that uses `Sym` as its underlying symbol type.
pub type DefaultStringInterner = StringInterner<DefaultSymbol>;

/// Caches strings efficiently, with minimal memory footprint and associates them with unique symbols.
/// These symbols allow constant time comparisons and look-ups to the underlying interned strings.
#[derive(Debug, Eq)]
pub struct StringInterner<S, H = RandomState>
where
    S: Symbol,
    H: BuildHasher,
{
    map: HashMap<PinnedStr, S, H>,
    values: Vec<Pin<Box<str>>>,
}

impl<S, H> PartialEq for StringInterner<S, H>
where
    S: Symbol,
    H: BuildHasher,
{
    fn eq(&self, rhs: &Self) -> bool {
        self.len() == rhs.len() && self.values == rhs.values
    }
}

impl Default for StringInterner<DefaultSymbol, RandomState> {
    #[inline]
    fn default() -> Self {
        StringInterner::new()
    }
}

// Should be manually cloned.
// See <https://github.com/Robbepop/string-interner/issues/9>.
impl<S, H> Clone for StringInterner<S, H>
where
    S: Symbol,
    H: Clone + BuildHasher,
{
    fn clone(&self) -> Self {
        let values = self.values.clone();
        let mut map =
            HashMap::with_capacity_and_hasher(values.len(), self.map.hasher().clone());
        // Recreate `InternalStrRef` from the newly cloned `Box<str>`s.
        // Use `extend()` to avoid `H: Default` trait bound required by `FromIterator for HashMap`.
        map.extend(
            values
                .iter()
                .enumerate()
                .map(|(i, s)| (PinnedStr::from_str(s), S::from_usize(i))),
        );
        Self { values, map }
    }
}

// About `Send` and `Sync` impls for `StringInterner`
// --------------------------------------------------
//
// tl;dr: Automation of Send+Sync impl was prevented by `InternalStrRef`
// being an unsafe abstraction and thus prevented Send+Sync default derivation.
//
// These implementations are safe due to the following reasons:
//  - `InternalStrRef` cannot be used outside `StringInterner`.
//  - Strings stored in `StringInterner` are not mutable.
//  - Iterator invalidation while growing the underlying `Vec<Box<str>>` is prevented by
//    using an additional indirection to store strings.
unsafe impl<S, H> Send for StringInterner<S, H>
where
    S: Symbol + Send,
    H: BuildHasher,
{
}
unsafe impl<S, H> Sync for StringInterner<S, H>
where
    S: Symbol + Sync,
    H: BuildHasher,
{
}

impl<S> StringInterner<S>
where
    S: Symbol,
{
    /// Creates a new empty `StringInterner`.
    #[inline]
    pub fn new() -> StringInterner<S, RandomState> {
        StringInterner {
            map: HashMap::new(),
            values: Vec::new(),
        }
    }

    /// Creates a new `StringInterner` with the given initial capacity.
    #[inline]
    pub fn with_capacity(cap: usize) -> Self {
        StringInterner {
            map: HashMap::with_capacity(cap),
            values: Vec::with_capacity(cap),
        }
    }

    /// Returns the number of elements the `StringInterner` can hold without reallocating.
    #[inline]
    pub fn capacity(&self) -> usize {
        std::cmp::min(self.map.capacity(), self.values.capacity())
    }

    /// Reserves capacity for at least `additional` more elements to be interned into `self`.
    ///
    /// The collection may reserve more space to avoid frequent allocations.
    /// After calling `reserve`, capacity will be greater than or equal to `self.len() + additional`.
    /// Does nothing if capacity is already sufficient.
    #[inline]
    pub fn reserve(&mut self, additional: usize) {
        self.map.reserve(additional);
        self.values.reserve(additional);
    }
}

impl<S, H> StringInterner<S, H>
where
    S: Symbol,
    H: BuildHasher,
{
    /// Creates a new empty `StringInterner` with the given hasher.
    #[inline]
    pub fn with_hasher(hash_builder: H) -> StringInterner<S, H> {
        StringInterner {
            map: HashMap::with_hasher(hash_builder),
            values: Vec::new(),
        }
    }

    /// Creates a new empty `StringInterner` with the given initial capacity and the given hasher.
    #[inline]
    pub fn with_capacity_and_hasher(cap: usize, hash_builder: H) -> StringInterner<S, H> {
        StringInterner {
            map: HashMap::with_hasher(hash_builder),
            values: Vec::with_capacity(cap),
        }
    }

    /// Interns the given value.
    ///
    /// Returns a symbol to access it within this interner.
    ///
    /// This either copies the contents of the string (e.g. for str)
    /// or moves them into this interner (e.g. for String).
    #[inline]
    pub fn get_or_intern<T>(&mut self, val: T) -> S
    where
        T: Into<String> + AsRef<str>,
    {
        match self.map.get(&PinnedStr::from_str(val.as_ref())) {
            Some(&sym) => sym,
            None => self.intern(val),
        }
    }

    /// Interns the given value and ignores collissions.
    ///
    /// Returns a symbol to access it within this interner.
    fn intern<T>(&mut self, new_val: T) -> S
    where
        T: Into<String> + AsRef<str>,
    {
        let new_id: S = self.make_symbol();
        let new_boxed_val = Pin::new(new_val.into().into_boxed_str());
        let new_ref = PinnedStr::from_pin(new_boxed_val.as_ref());
        self.values.push(new_boxed_val);
        self.map.insert(new_ref, new_id);
        new_id
    }

    /// Creates a new symbol for the current state of the interner.
    fn make_symbol(&self) -> S {
        S::from_usize(self.len())
    }

    /// Returns the string slice associated with the given symbol if available,
    /// otherwise returns `None`.
    #[inline]
    pub fn resolve(&self, symbol: S) -> Option<&str> {
        self.values
            .get(symbol.to_usize())
            .map(|boxed_str| boxed_str.as_ref().get_ref())
    }

    /// Returns the string associated with the given symbol.
    ///
    /// # Note
    ///
    /// This does not check whether the given symbol has an associated string
    /// for the given string interner instance.
    ///
    /// # Safety
    ///
    /// This will result in undefined behaviour if the given symbol
    /// has no associated string for this interner instance.
    #[inline]
    pub unsafe fn resolve_unchecked(&self, symbol: S) -> &str {
        self.values
            .get_unchecked(symbol.to_usize())
            .as_ref()
            .get_ref()
    }

    /// Returns the symbol associated with the given string for this interner
    /// if existent, otherwise returns `None`.
    #[inline]
    pub fn get<T>(&self, val: T) -> Option<S>
    where
        T: AsRef<str>,
    {
        self.map.get(&PinnedStr::from_str(val.as_ref())).cloned()
    }

    /// Returns the number of uniquely interned strings within this interner.
    #[inline]
    pub fn len(&self) -> usize {
        self.values.len()
    }

    /// Returns true if the string interner holds no elements.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Returns an iterator over the interned strings.
    #[inline]
    pub fn iter(&self) -> Iter<S> {
        Iter::new(self)
    }

    /// Returns an iterator over all intern indices and their associated strings.
    #[inline]
    pub fn iter_values(&self) -> Values<S> {
        Values::new(self)
    }

    /// Shrinks the capacity of the interner as much as possible.
    pub fn shrink_to_fit(&mut self) {
        self.map.shrink_to_fit();
        self.values.shrink_to_fit();
    }
}

impl<T, S> FromIterator<T> for StringInterner<S>
where
    S: Symbol,
    T: Into<String> + AsRef<str>,
{
    fn from_iter<I>(iter: I) -> Self
    where
        I: IntoIterator<Item = T>,
    {
        let iter = iter.into_iter();
        let mut interner = StringInterner::with_capacity(iter.size_hint().0);
        interner.extend(iter);
        interner
    }
}

impl<T, S> std::iter::Extend<T> for StringInterner<S>
where
    S: Symbol,
    T: Into<String> + AsRef<str>,
{
    fn extend<I>(&mut self, iter: I)
    where
        I: IntoIterator<Item = T>,
    {
        for s in iter {
            self.get_or_intern(s);
        }
    }
}

/// Iterator over the pairs of associated symbols and interned strings for a `StringInterner`.
pub struct Iter<'a, S> {
    iter: iter::Enumerate<slice::Iter<'a, Pin<Box<str>>>>,
    mark: marker::PhantomData<S>,
}

impl<'a, S> Iter<'a, S>
where
    S: Symbol + 'a,
{
    /// Creates a new iterator for the given StringIterator over pairs of
    /// symbols and their associated interned string.
    #[inline]
    fn new<H>(interner: &'a StringInterner<S, H>) -> Self
    where
        H: BuildHasher,
    {
        Iter {
            iter: interner.values.iter().enumerate(),
            mark: marker::PhantomData,
        }
    }
}

impl<'a, S> Iterator for Iter<'a, S>
where
    S: Symbol + 'a,
{
    type Item = (S, &'a str);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        self.iter
            .next()
            .map(|(num, boxed_str)| (S::from_usize(num), boxed_str.as_ref().get_ref()))
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.iter.size_hint()
    }
}

/// Iterator over the interned strings of a `StringInterner`.
pub struct Values<'a, S>
where
    S: Symbol + 'a,
{
    iter: slice::Iter<'a, Pin<Box<str>>>,
    mark: marker::PhantomData<S>,
}

impl<'a, S> Values<'a, S>
where
    S: Symbol + 'a,
{
    /// Creates a new iterator for the given StringIterator over its interned strings.
    #[inline]
    fn new<H>(interner: &'a StringInterner<S, H>) -> Self
    where
        H: BuildHasher,
    {
        Values {
            iter: interner.values.iter(),
            mark: marker::PhantomData,
        }
    }
}

impl<'a, S> Iterator for Values<'a, S>
where
    S: Symbol + 'a,
{
    type Item = &'a str;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        self.iter
            .next()
            .map(|boxed_str| boxed_str.as_ref().get_ref())
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.iter.size_hint()
    }
}

impl<S, H> iter::IntoIterator for StringInterner<S, H>
where
    S: Symbol,
    H: BuildHasher,
{
    type Item = (S, String);
    type IntoIter = IntoIter<S>;

    fn into_iter(self) -> Self::IntoIter {
        IntoIter {
            iter: self.values.into_iter().enumerate(),
            mark: marker::PhantomData,
        }
    }
}

impl<'a, S, H> iter::IntoIterator for &'a StringInterner<S, H>
where
    S: Symbol,
    H: BuildHasher,
{
    type Item = (S, &'a str);
    type IntoIter = Iter<'a, S>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

/// Iterator over the pairs of associated symbol and strings.
///
/// Consumes the `StringInterner` upon usage.
pub struct IntoIter<S>
where
    S: Symbol,
{
    iter: iter::Enumerate<vec::IntoIter<Pin<Box<str>>>>,
    mark: marker::PhantomData<S>,
}

impl<S> Iterator for IntoIter<S>
where
    S: Symbol,
{
    type Item = (S, String);

    fn next(&mut self) -> Option<Self::Item> {
        self.iter.next().map(|(num, boxed_str)| {
            (S::from_usize(num), Pin::into_inner(boxed_str).into_string())
        })
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.iter.size_hint()
    }
}
