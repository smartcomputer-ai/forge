#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum PatchOperation {
    AddFile {
        path: String,
        lines: Vec<String>,
    },
    DeleteFile {
        path: String,
    },
    UpdateFile {
        path: String,
        move_to: Option<String>,
        hunks: Vec<PatchHunk>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PatchHunk {
    pub(crate) header: String,
    pub(crate) lines: Vec<PatchHunkLine>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum PatchHunkLine {
    Context(String),
    Delete(String),
    Add(String),
    EndOfFile,
}
