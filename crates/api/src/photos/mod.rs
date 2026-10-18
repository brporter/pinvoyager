//! The photo pipeline.
//!
//! Upload → validate → store → extract EXIF → derive variants → serve.
//!
//! Three invariants govern everything here, and each is a decision rather than
//! a preference:
//!
//! **D6 — variants carry no metadata.** Published voyages are public, so a served
//! image is a public artifact. If a user picks coarse granularity and the
//! served file still carries its original GPS EXIF, the setting is decorative
//! and leaks exact coordinates to anyone who downloads it. Re-encoding is not
//! the same as stripping; the strip is asserted with `exiftool` in the tests
//! because encoders differ on what they carry through and a dependency upgrade
//! can change that underneath you.
//!
//! **D7 — nothing is served from disk directly.** Every read goes through a
//! handler that sets a server-determined `Content-Type`, `nosniff`, and a
//! restrictive `Content-Disposition`. The failure being avoided is a stored
//! XSS: a user uploads a file the browser sniffs as HTML and it executes on our
//! origin against another user's session.
//!
//! **D8 — image work never runs on a runtime worker thread.** Decode, resize,
//! encode and hashing are all CPU-bound. A fifty-file batch run inline stalls
//! the runtime and times out unrelated requests. Everything goes through
//! [`crate::blocking::run_blocking`].

pub mod cleanup;
pub mod decode;
pub mod exif;
pub mod rate_limit;
pub mod storage;
pub(crate) mod temporary;
pub mod validate;
pub mod variants;
pub mod worker;

pub use validate::ImageFormat;
pub use variants::VariantKind;

#[derive(Debug, thiserror::Error)]
pub enum PhotoError {
    #[error("upload exceeds the size limit")]
    TooLarge,

    #[error("not an image file")]
    NotAnImage,

    #[error("image dimensions exceed the limit")]
    TooManyPixels,

    #[error("image could not be decoded")]
    Undecodable(String),

    #[error("image could not be encoded")]
    Unencodable(String),

    #[error("storage error")]
    Storage(#[source] std::io::Error),
}

pub mod account_storage;
