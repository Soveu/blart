#![cfg_attr(
    feature = "nightly",
    feature(
        impl_trait_in_assoc_type,
        maybe_uninit_slice,
        maybe_uninit_uninit_array,
        const_maybe_uninit_uninit_array,
        maybe_uninit_array_assume_init,
        slice_ptr_get,
        hasher_prefixfree_extras,
        new_uninit,
        core_intrinsics,
        strict_provenance,
        portable_simd
    )
)]
#![cfg_attr(feature = "nightly", allow(incomplete_features, internal_features))]
#![allow(clippy::type_complexity, unstable_name_collisions)]
#![deny(
    missing_docs,
    clippy::missing_safety_doc,
    unsafe_op_in_unsafe_fn,
    deprecated_in_future,
    rustdoc::broken_intra_doc_links,
    rustdoc::bare_urls,
    rustdoc::invalid_codeblock_attributes
)]
#![doc(
    html_playground_url = "https://play.rust-lang.org/",
    test(attr(deny(warnings)))
)]

//! Adaptive radix trie implementation
//!
//! # References
//!
//!  - Leis, V., Kemper, A., & Neumann, T. (2013, April). The adaptive radix
//!    tree: ARTful indexing for main-memory databases. In 2013 IEEE 29th
//!    International Conference on Data Engineering (ICDE) (pp. 38-49). IEEE.
//!    [Link to PDF][ART paper]
//!
//! [ART paper]: https://www-db.in.tum.de/~leis/papers/ART.pdf

mod bytes;
mod collections;
mod nodes;
mod tagged_pointer;

#[doc(hidden)]
pub mod tests_common;

mod rust_nightly_apis;

pub use bytes::*;
pub use collections::*;
pub use nodes::{visitor, *};

/// Standard ART type. The default is for variable key lengths, with 16 byte
/// prefix
pub type TreeMap<K, V> = RawTreeMap<K, V, 16>;

#[doc = include_str!("../README.md")]
#[cfg(doctest)]
pub struct ReadmeDoctests;
