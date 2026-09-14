//! Archives come in various flavors, and the specific variant you'll need to use depends on which game you're working with. Learn more by choosing one of [`tes4`] or [`fo4`].
//!
//! If you are uncertain of the origins of your archive, then you may use [`guess_format`] to find a starting point.
//!
//! # A note on strings
//! The Creation Engine mishandles Unicode and has bugs with extended-ASCII characters, so all strings are binary strings with no encoding (see [`BStr`] and [`BString`]). Archives usually use the writing machine's system code page (Windows-1252 for English copies, Windows-1251 for Russian), but that isn't guaranteed.

#![warn(
    clippy::pedantic,
    clippy::single_char_lifetime_names,
    clippy::std_instead_of_core
)]
#![allow(
    unknown_lints,
    clippy::enum_glob_use,
    clippy::missing_errors_doc,
    clippy::struct_field_names
)]

mod cc;
mod containers;
mod derive;
pub mod fo4;
pub mod fs_index;
mod guess;
mod hashing;
pub mod incremental;
mod io;
mod mod_pack;
mod pack;
mod pack_fo4_stream;
mod protocols;
mod ps_audio;
pub mod python;
pub mod tes4;
mod worker_pool;

pub use guess::{FileFormat, guess_format};
pub use python::{list_archive_files, register_module};

/// Makes a shallow copy of the input.
///
/// The lifetime of the result is tied to the input buffer.
pub struct Borrowed<'borrow>(pub &'borrow [u8]);

/// Makes a deep copy of the input.
///
/// The lifetime of the result is independent of the input buffer.
pub struct Copied<'copy>(pub &'copy [u8]);

mod private {
    pub trait Sealed {}
}

use private::Sealed;

/// A trait that enables reading from various sources.
pub trait Reader<T>: Sealed {
    type Error;
    type Item;

    /// Reads an instance of `Self::Item` from the given source.
    fn read(source: T) -> core::result::Result<Self::Item, Self::Error>;
}

/// A trait that creates an optionally compressed container using the given value.
pub trait CompressableFrom<T>: Sealed {
    /// Makes a compressed instance of `Self` using the given data.
    #[must_use]
    fn from_compressed(value: T, decompressed_len: usize) -> Self;

    /// Makes a decompressed instance of `Self` using the given data.
    #[must_use]
    fn from_decompressed(value: T) -> Self;
}

/// Indicates whether the operation should finish by compressing the data or not.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum CompressionResult {
    /// The data will finish in a compressed state.
    Compressed,
    /// The data will finish in a decompressed state.
    #[default]
    Decompressed,
}

/// A trait that enables reading from various sources, with configuration options.
pub trait ReaderWithOptions<T>: Sealed {
    type Error;
    type Item;
    type Options;

    /// Reads an instance of `Self::Item` from the given source, using the given options.
    fn read(source: T, options: &Self::Options) -> core::result::Result<Self::Item, Self::Error>;
}

pub use bstr::{BStr, BString, ByteSlice, ByteVec};

/// Convenience using statements for traits that are needed to work with the library.
pub mod prelude {
    pub use crate::{CompressableFrom as _, Reader as _, ReaderWithOptions as _};
}
