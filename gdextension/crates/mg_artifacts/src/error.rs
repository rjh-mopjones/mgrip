use std::path::PathBuf;

#[derive(Debug)]
pub enum ArtifactError {
    Io { context: String, source: std::io::Error },
    Bincode { context: String, source: Box<bincode::ErrorKind> },
    RonSerialize { context: String, source: ron::Error },
    RonDeserialize { context: String, source: ron::error::SpannedError },
    Image { context: String, source: image::ImageError },
    NotFound { kind: String, tag: String },
    FileNotFound { path: PathBuf },
    InvalidTag { tag: String, reason: &'static str },
    NoHomeDirectory,
}

impl std::fmt::Display for ArtifactError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io { context, source } => write!(f, "IO error ({context}): {source}"),
            Self::Bincode { context, source } => write!(f, "Bincode error ({context}): {source}"),
            Self::RonSerialize { context, source } => write!(f, "RON serialize error ({context}): {source}"),
            Self::RonDeserialize { context, source } => write!(f, "RON deserialize error ({context}): {source}"),
            Self::Image { context, source } => write!(f, "Image error ({context}): {source}"),
            Self::NotFound { kind, tag } => write!(f, "{kind} artifact '{tag}' not found"),
            Self::FileNotFound { path } => write!(f, "File not found: {}", path.display()),
            Self::InvalidTag { tag, reason } => write!(f, "Invalid tag '{tag}': {reason}"),
            Self::NoHomeDirectory => write!(f, "Could not determine home directory"),
        }
    }
}

impl std::error::Error for ArtifactError {}
