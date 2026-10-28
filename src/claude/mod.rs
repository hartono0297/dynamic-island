pub mod hook;
pub mod install;
pub mod pipe_server;

/// Local-only pipe the island listens on and `--hook` writes one JSON line to.
const PIPE_NAME: &str = r"\\.\pipe\vale";
