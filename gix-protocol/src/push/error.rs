/// The error returned by [`push()`](crate::push()).
#[derive(Debug, thiserror::Error)]
#[allow(missing_docs)]
pub enum Error {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Transport(#[from] gix_transport::client::Error),
    #[error("the server does not support the '{feature}' capability which is required: {description}")]
    MissingServerCapability {
        feature: &'static str,
        description: &'static str,
    },
    #[error("atomic push was requested but the server does not support the 'atomic' capability")]
    AtomicNotSupported,
    #[error("the server rejected the pack: {reason}")]
    UnpackFailed { reason: String },
    #[error("failed to parse the push response from the server")]
    ResponseParse(#[from] crate::push::response::Error),
    #[error("failed to generate the pack to send")]
    PackGeneration(#[from] Box<dyn std::error::Error + Send + Sync + 'static>),
    #[error("no commands to send to the server")]
    NoCommands,
}

impl crate::transport::IsSpuriousError for Error {
    fn is_spurious(&self) -> bool {
        match self {
            Error::Io(err) => {
                err.kind() == std::io::ErrorKind::ConnectionReset || err.kind() == std::io::ErrorKind::ConnectionAborted
            }
            Error::Transport(err) => err.is_spurious(),
            _ => false,
        }
    }
}
