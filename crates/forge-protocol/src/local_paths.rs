//! One CWD-independent local socket convention shared by Core, CLI, and Supervisor.

use std::{env, path::PathBuf};

/// Resolves the directory that owns the local Core sockets.
///
/// Runtime sockets prefer the XDG runtime directory. A persistent per-user
/// state directory is the fallback for a daemon without a login session. The
/// final absolute `/tmp` fallback is deliberately shared rather than relative:
/// the owner-only directory checks at the server boundary make a collision fail
/// safely, while Core, CLI, and Supervisor still agree on one location.
#[must_use]
pub fn default_runtime_directory() -> PathBuf {
    runtime_directory_from(
        absolute_environment_path("XDG_RUNTIME_DIR"),
        absolute_environment_path("XDG_STATE_HOME"),
        absolute_environment_path("HOME"),
    )
}

/// Private control socket used only by the owner CLI to request a UI login code.
#[must_use]
pub fn default_ui_control_socket_path() -> PathBuf {
    default_runtime_directory().join("ui-control.sock")
}

fn absolute_environment_path(name: &str) -> Option<PathBuf> {
    env::var_os(name)
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
}

fn runtime_directory_from(
    runtime_directory: Option<PathBuf>,
    state_home: Option<PathBuf>,
    home: Option<PathBuf>,
) -> PathBuf {
    runtime_directory
        .map(|directory| directory.join("forge"))
        .or_else(|| state_home.map(|directory| directory.join("forge/run")))
        .or_else(|| home.map(|directory| directory.join(".local/state/forge/run")))
        .unwrap_or_else(|| PathBuf::from("/tmp/forge"))
}

#[cfg(test)]
mod tests {
    use super::runtime_directory_from;
    use std::path::PathBuf;

    #[test]
    fn runtime_directory_precedes_state_and_home() {
        let directory = runtime_directory_from(
            Some(PathBuf::from("/run/user/1000")),
            Some(PathBuf::from("/state")),
            Some(PathBuf::from("/home/forge")),
        );

        assert_eq!(directory, PathBuf::from("/run/user/1000/forge"));
    }

    #[test]
    fn state_fallback_is_absolute_and_cwd_independent() {
        let directory = runtime_directory_from(
            None,
            Some(PathBuf::from("/state")),
            Some(PathBuf::from("/home/forge")),
        );

        assert_eq!(directory, PathBuf::from("/state/forge/run"));
    }

    #[test]
    fn home_and_tmp_fallbacks_are_absolute() {
        assert_eq!(
            runtime_directory_from(None, None, Some(PathBuf::from("/home/forge"))),
            PathBuf::from("/home/forge/.local/state/forge/run")
        );
        assert_eq!(
            runtime_directory_from(None, None, None),
            PathBuf::from("/tmp/forge")
        );
    }
}
