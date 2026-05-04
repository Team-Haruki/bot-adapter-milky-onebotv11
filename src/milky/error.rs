use thiserror::Error;

#[derive(Debug, Error)]
pub enum MilkyClientError {
    #[error("milky sdk: {0}")]
    Sdk(#[from] milky_rust_sdk::MilkyError),
    #[error("bad segment: {0}")]
    BadSegment(String),
    #[error("not connected")]
    NotConnected,
}
