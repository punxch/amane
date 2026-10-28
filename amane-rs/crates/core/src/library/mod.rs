//! 媒体库登记与扫描.

pub mod model;
pub mod repository;
pub mod rules;

pub use model::{LibraryFile, LibraryRoot, ScanDiff, ScanError, ScanKind, UnwantedRule};
pub use repository::{KindCounts, LibraryError, LibraryRepository};
pub use rules::{
    DEFAULT_MEDIA_EXTENSIONS, ScanOutcome, ScanRules, TRASH_DIRNAME, normalize_extension,
    scan_directory,
};
