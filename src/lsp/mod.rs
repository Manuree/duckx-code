//! Language Server Protocol support. `App` queues [`LspRequest`]s and applies
//! [`LspEvent`]s; [`client`] runs the servers on a tokio task off the UI thread.

pub mod client;
pub mod detect;
pub mod diagnostics;
pub mod plain;
pub mod position;
pub mod protocol;
pub mod registry;
pub mod semantic;
pub mod transport;
pub mod uri;

pub use protocol::{Diagnostic, LspEvent, LspRequest, ServerStatus, Severity, TextChange};

/// Files larger than this are not sent to a language server.
pub const MAX_DOCUMENT_BYTES: usize = 8 * 1024 * 1024;
