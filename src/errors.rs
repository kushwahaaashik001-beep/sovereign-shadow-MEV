use alloy_primitives::B256;
use thiserror::Error;

#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum DecodingError {
    #[error("Input data too short")]
    InputTooShort,
    #[error("Unknown selector: {0:?}")]
    UnknownSelector([u8; 4]),
    #[error("Invalid data length for selector {0}")]
    InvalidDataLength(String),
    #[error("Failed to read dynamic data at offset {0}")]
    DynamicDataReadFailed(usize),
    #[error("Invalid path length: {0}")]
    InvalidPathLength(usize),
    #[error("Unsupported Universal Router command: {0}")]
    UnsupportedUniversalRouterCommand(u8),
    #[error("Recursive decoding depth exceeded in Multicall")]
    MulticallDepthExceeded,
    #[error("Invalid data in nested component of transaction {0:?}")]
    InvalidNestedData(B256),
    #[error("Address parsing failed from data slice")]
    AddressParsingFailed,
    #[error("U256 parsing failed from data slice")]
    U256ParsingFailed,
    #[error("Usize parsing failed from data slice")]
    UsizeParsingFailed,
    #[error("Attempted to decode a transaction with no recipient")]
    NoRecipient,
}
