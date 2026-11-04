//! Host behavior injected by the shell; never inferred by the core.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HostKind {
    Unix,
    Windows,
}
