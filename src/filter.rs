//! Safe filtering logic — no FFI here.
//!
//! `is_nix` decides whether an executable belongs to the nix world (and
//! must keep its environment); `filter_env_vars` rebuilds the environment
//! without the glibc-only variables. Both are pure/testable.

use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

/// Variables stripped from the environment of non-nix children.
///
/// These are exported by nixGL (nix mesa/libglvnd stack) for the DMS/quickshell
/// process; inherited by musl children they cause relocation failures such as
/// `Error relocating .../libGL.so.1: __memcpy_chk: symbol not found`.
pub const STRIP_VARS: &[&str] = &[
    "LD_LIBRARY_PATH",
    "LD_PRELOAD",
    "__EGL_VENDOR_LIBRARY_FILENAMES",
    "GBM_BACKENDS_PATH",
    "LIBGL_DRIVERS_PATH",
    "LIBVA_DRIVERS_PATH",
];

const NIX_STORE_PREFIX: &[u8] = b"/nix/store/";
const NIX_PROFILE_MARKER: &[u8] = b"/.nix-profile/";

/// True if the executable belongs to the nix world and must keep its
/// environment: `/nix/store/...` paths, `~/.nix-profile/...` paths, and any
/// symlink that resolves into the store (e.g. `~/.nix-profile/bin/dms`).
pub fn is_nix(executable: &OsStr) -> bool {
    let bytes = executable.as_bytes();
    if bytes.starts_with(NIX_STORE_PREFIX) {
        return true;
    }
    if bytes.windows(NIX_PROFILE_MARKER.len()).any(|w| w == NIX_PROFILE_MARKER) {
        return true;
    }
    // Symlink resolution (~/.nix-profile/bin/dms -> /nix/store/...).
    matches!(std::fs::canonicalize(Path::new(executable)), Ok(ref real)
        if real.as_os_str().as_bytes().starts_with(NIX_STORE_PREFIX))
}

/// True if the entry is `KEY=...` for one of [`STRIP_VARS`].
fn is_stripped_var(entry: &OsStr) -> bool {
    STRIP_VARS.iter().any(|k| {
        let kb = k.as_bytes();
        let eb = entry.as_bytes();
        eb.len() > kb.len() && eb.starts_with(kb) && eb[kb.len()] == b'='
    })
}

/// Rebuild the environment without the stripped variables. Order preserved.
pub fn filter_env_vars<'a, I>(env: I) -> Vec<&'a OsStr>
where
    I: IntoIterator<Item = &'a OsStr>,
{
    env.into_iter().filter(|e| !is_stripped_var(e)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;
    use std::os::unix::fs::symlink;

    fn os(s: &str) -> &OsStr {
        OsStr::new(s)
    }

    #[test]
    fn nix_store_path_keeps_env() {
        assert!(is_nix(os("/nix/store/abc-def/bin/dms")));
        assert!(is_nix(os("/nix/store/xyz")));
    }

    #[test]
    fn nix_profile_path_keeps_env() {
        assert!(is_nix(os("/home/gab/.nix-profile/bin/dms")));
        assert!(is_nix(os("/home/gab/.nix-profile/bin/qs")));
        // marker only — no store prefix needed (C shim: strstr)
        assert!(is_nix(os("/x/.nix-profile/y")));
    }

    #[test]
    fn system_paths_do_not_keep_env() {
        assert!(!is_nix(os("/usr/bin/ls")));
        assert!(!is_nix(os("/bin/sh")));
    }

    #[test]
    fn nonexistent_path_is_not_nix_and_does_not_panic() {
        assert!(!is_nix(os("/nonexistent-execfilter-xyz")));
    }

    #[test]
    fn symlink_into_store_is_nix() {
        let dir = std::env::temp_dir().join(format!("execfilter-test-{}", std::process::id()));
        let target = dir.join("store-link");
        let link = dir.join("profile-link");
        let _ = std::fs::create_dir_all(&dir);
        let _ = symlink(dir.join("nonexistent-store-target"), &target);
        let _ = std::fs::remove_file(&link);
        // real /nix/store dir if present, else a temp dir — either way the
        // symlink resolves to something that is NOT /nix/store => false,
        // proving canonicalize is exercised without false positives.
        let store = std::path::Path::new("/nix/store");
        if store.is_dir() {
            let _ = std::fs::remove_file(&link);
            let _ = symlink(store, &link);
            assert!(is_nix(link.as_os_str()));
        }
        assert!(!is_nix(target.as_os_str()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn filter_strips_glibc_only_vars() {
        let env = [
            OsString::from("PATH=/usr/bin:/bin"),
            OsString::from("LD_LIBRARY_PATH=/nix/store/xyz/lib"),
            OsString::from("__EGL_VENDOR_LIBRARY_FILENAMES=/nix/store/gles.json"),
            OsString::from("GBM_BACKENDS_PATH=/nix/store/gbm"),
            OsString::from("LIBGL_DRIVERS_PATH=/nix/store/dri"),
            OsString::from("LIBVA_DRIVERS_PATH=/nix/store/va"),
            OsString::from("LD_PRELOAD=/nix/store/shim.so"),
            OsString::from("HOME=/home/gab"),
        ];
        let kept: Vec<String> = filter_env_vars(env.iter().map(OsString::as_os_str))
            .iter()
            .map(|s| s.to_string_lossy().into_owned())
            .collect();
        assert_eq!(kept, vec!["PATH=/usr/bin:/bin", "HOME=/home/gab"]);
    }

    #[test]
    fn filter_is_prefix_exact_not_partial() {
        let env = [
            OsString::from("LD_LIBRARY_PATH=/keep/me"),   // stripped
            OsString::from("LD_LIBRARY_PATH_EXTRA=/keep"), // NOT a strip var
            OsString::from("LD_LIBRARY_PATH"),             // no '=' → kept
        ];
        let kept: Vec<String> = filter_env_vars(env.iter().map(OsString::as_os_str))
            .iter()
            .map(|s| s.to_string_lossy().into_owned())
            .collect();
        assert_eq!(kept, vec!["LD_LIBRARY_PATH_EXTRA=/keep", "LD_LIBRARY_PATH"]);
    }

    #[test]
    fn filter_empty_env() {
        let env: [OsString; 0] = [];
        assert!(filter_env_vars(env.iter().map(OsString::as_os_str)).is_empty());
    }
}
