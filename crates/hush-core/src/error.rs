use core::fmt;

/// Every way the core can refuse an input. Deliberately coarse: callers show a message and drop
/// the input, they never branch on cryptographic details.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    /// The bytes do not parse (wrong length, version, tag or encoding).
    Malformed,
    /// Authentication failed: wrong key, tampered data or wrong passphrase.
    Decrypt,
    /// A signature does not verify.
    BadSignature,
    /// A self-certifying identifier does not match the keys presented with it.
    IdMismatch,
    /// The input is outside its validity window (a link, envelope or bundle that has expired).
    Expired,
    /// The input was already seen.
    Replay,
    /// The call does not fit the current state of the state machine.
    BadState,
    /// A counter jumped further than the skipped-key window allows, or a bounded buffer is full.
    TooFarAhead,
    /// A proof of work is missing or too weak.
    WeakProofOfWork,
    /// A parameter is out of range (e.g. Argon2 costs).
    InvalidParameter,
    /// Made by an older version of the protocol that is no longer accepted (a version 1 link).
    Outdated,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Error::Malformed => "malformed input",
            Error::Decrypt => "decryption failed",
            Error::BadSignature => "bad signature",
            Error::IdMismatch => "identifier does not match the keys",
            Error::Expired => "expired",
            Error::Replay => "replayed",
            Error::BadState => "unexpected in the current state",
            Error::TooFarAhead => "too many skipped messages",
            Error::WeakProofOfWork => "proof of work too weak",
            Error::InvalidParameter => "invalid parameter",
            Error::Outdated => "made by an older version of hush",
        })
    }
}

impl core::error::Error for Error {}

pub type Result<T> = core::result::Result<T, Error>;
