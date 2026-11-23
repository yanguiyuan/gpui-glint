pub mod storage;
mod workspace;

pub use workspace::{
    CloseDocument, MarkdownWorkspace, NewDocument, OpenDocument, QuitWorkspace, SaveDocument,
    bind_workspace_keys, open_external,
};
