//! execfilter — LD_PRELOAD shim (Rust core + C varargs hooks).
//!
//! Interposes the exec family and strips nixGL/glibc-only environment
//! variables from the environment of child processes that are NOT nix
//! binaries. Children under `/nix/store` or `~/.nix-profile` keep the full
//! environment so nix apps keep their GL stack.
//!
//! Hook layout:
//! - Rust (`src/lib.rs`): execve, execv, execvp, execvpe, posix_spawn,
//!   posix_spawnp
//! - C (`src/execl_shim.c`): execl, execlp — C-variadic definitions are
//!   nightly-only in Rust (`std::ffi::VaList`), so these two stay in C and
//!   route through our own exported execv/execvp (glibc implements them on
//!   top of the internal `__execve`, which bypasses an `execve`-only hook).
//!
//! Constraint parity with the original C shim:
//! - must be a GLIBC shared object (it is preloaded into the glibc nix
//!   process tree) — never musl;
//! - the execv/execvp hooks swap the global `environ` around the real call
//!   (not thread-safe, same as upstream);
//! - hook bodies must not panic: `panic = "abort"` is set for release, and
//!   every fallible operation falls back to passing the original call
//!   through untouched. Deviation from C: a NULL `envp` is passed through
//!   instead of being rebuilt into an empty environment (POSIX UB either
//!   way; pass-through is the safer choice).

mod filter;

use std::ffi::{c_char, CStr, CString, OsStr, OsString};
use std::os::unix::ffi::OsStrExt;
use std::sync::OnceLock;

use filter::{filter_env_vars, is_nix};

extern "C" {
    static mut environ: *mut *mut c_char;
}

type ExecveFn = unsafe extern "C" fn(
    *const c_char,
    *const *const c_char,
    *const *const c_char,
) -> i32;
type ExecFn = unsafe extern "C" fn(*const c_char, *const *const c_char) -> i32;
type SpawnFn = unsafe extern "C" fn(
    *mut libc::pid_t,
    *const c_char,
    *const libc::posix_spawn_file_actions_t,
    *const libc::posix_spawnattr_t,
    *const *const c_char,
    *const *const c_char,
) -> i32;

/// Resolve the real libc function via RTLD_NEXT (cached per call site).
/// Aborts (no unwinding) if the symbol cannot be resolved — calling a null
/// function pointer would be worse.
macro_rules! real {
    ($name:expr, $ty:ty) => {{
        static REAL: OnceLock<usize> = OnceLock::new();
        let p = *REAL.get_or_init(|| unsafe { libc::dlsym(libc::RTLD_NEXT, $name) } as usize);
        if p == 0 {
            std::process::abort();
        }
        unsafe { std::mem::transmute_copy::<usize, $ty>(&p) }
    }};
}

/// Borrow the path as [`OsStr`]; None on NULL.
unsafe fn raw_os_str(p: *const c_char) -> Option<&'static OsStr> {
    if p.is_null() {
        return None;
    }
    Some(OsStr::from_bytes(unsafe { CStr::from_ptr(p) }.as_bytes()))
}

/// Does the path belong to the nix world? NULL → false (never panics).
unsafe fn path_is_nix(p: *const c_char) -> bool {
    unsafe { raw_os_str(p) }.map(is_nix).unwrap_or(false)
}

/// Copy the raw NULL-terminated `envp` into owned strings; None if NULL.
unsafe fn envp_to_strings(envp: *const *const c_char) -> Option<Vec<OsString>> {
    if envp.is_null() {
        return None;
    }
    let mut vars = Vec::new();
    let mut i = 0usize;
    loop {
        let p = unsafe { *envp.add(i) };
        if p.is_null() {
            break;
        }
        vars.push(unsafe { raw_os_str(p) }?.to_os_string());
        i += 1;
    }
    Some(vars)
}

/// Owned filtered environment + raw NULL-terminated pointer array.
/// None when envp is NULL or nothing needed stripping (pass-through).
unsafe fn filtered_envp(
    envp: *const *const c_char,
) -> Option<(Vec<CString>, Vec<*const c_char>)> {
    let vars = unsafe { envp_to_strings(envp) }?;
    let filtered = filter_env_vars(vars.iter().map(OsString::as_os_str));
    // Entries with an interior NUL cannot occur in a real environ; drop them.
    let owned: Vec<CString> = filtered
        .iter()
        .filter_map(|v| CString::new(v.as_bytes().to_vec()).ok())
        .collect();
    if owned.len() == vars.len() {
        return None;
    }
    let mut ptrs: Vec<*const c_char> = owned.iter().map(|s| s.as_ptr()).collect();
    ptrs.push(std::ptr::null());
    Some((owned, ptrs))
}

// ---------------------------------------------------------------------------
// execve / execvpe / posix_spawn family: envp is a parameter
// ---------------------------------------------------------------------------

/// # Safety
///
/// Standard `execve` contract; must only be reached through LD_PRELOAD
/// interposition with valid libc argument pointers.
#[no_mangle]
pub unsafe extern "C" fn execve(
    path: *const c_char,
    argv: *const *const c_char,
    envp: *const *const c_char,
) -> i32 {
    let real: ExecveFn = real!(c"execve".as_ptr(), ExecveFn);
    if unsafe { path_is_nix(path) } {
        return unsafe { real(path, argv, envp) };
    }
    match unsafe { filtered_envp(envp) } {
        Some((owned, ptrs)) => unsafe { real(path, argv, ptrs.as_ptr()) },
        None => unsafe { real(path, argv, envp) },
    }
}

/// # Safety
///
/// Standard `execvpe` contract; must only be reached through LD_PRELOAD
/// interposition with valid libc argument pointers.
#[no_mangle]
pub unsafe extern "C" fn execvpe(
    file: *const c_char,
    argv: *const *const c_char,
    envp: *const *const c_char,
) -> i32 {
    let real: ExecveFn = real!(c"execvpe".as_ptr(), ExecveFn);
    if unsafe { path_is_nix(file) } {
        return unsafe { real(file, argv, envp) };
    }
    match unsafe { filtered_envp(envp) } {
        Some((owned, ptrs)) => unsafe { real(file, argv, ptrs.as_ptr()) },
        None => unsafe { real(file, argv, envp) },
    }
}

/// # Safety
///
/// Standard `posix_spawn` contract; must only be reached through LD_PRELOAD
/// interposition with valid libc argument pointers.
#[no_mangle]
pub unsafe extern "C" fn posix_spawn(
    pid: *mut libc::pid_t,
    path: *const c_char,
    file_actions: *const libc::posix_spawn_file_actions_t,
    attrp: *const libc::posix_spawnattr_t,
    argv: *const *const c_char,
    envp: *const *const c_char,
) -> i32 {
    let real: SpawnFn = real!(c"posix_spawn".as_ptr(), SpawnFn);
    if unsafe { path_is_nix(path) } {
        return unsafe { real(pid, path, file_actions, attrp, argv, envp) };
    }
    match unsafe { filtered_envp(envp) } {
        Some((owned, ptrs)) => unsafe { real(pid, path, file_actions, attrp, argv, ptrs.as_ptr()) },
        None => unsafe { real(pid, path, file_actions, attrp, argv, envp) },
    }
}

/// # Safety
///
/// Standard `posix_spawnp` contract; must only be reached through LD_PRELOAD
/// interposition with valid libc argument pointers.
#[no_mangle]
pub unsafe extern "C" fn posix_spawnp(
    pid: *mut libc::pid_t,
    file: *const c_char,
    file_actions: *const libc::posix_spawn_file_actions_t,
    attrp: *const libc::posix_spawnattr_t,
    argv: *const *const c_char,
    envp: *const *const c_char,
) -> i32 {
    let real: SpawnFn = real!(c"posix_spawnp".as_ptr(), SpawnFn);
    if unsafe { path_is_nix(file) } {
        return unsafe { real(pid, file, file_actions, attrp, argv, envp) };
    }
    match unsafe { filtered_envp(envp) } {
        Some((owned, ptrs)) => unsafe { real(pid, file, file_actions, attrp, argv, ptrs.as_ptr()) },
        None => unsafe { real(pid, file, file_actions, attrp, argv, envp) },
    }
}

// ---------------------------------------------------------------------------
// execv / execvp: environ-based — swap the global around the real call
// (same approach, same thread-safety caveat, as the C shim)
// ---------------------------------------------------------------------------

/// Shared body for execv/execvp: filter the global `environ` unless the
/// target is a nix binary.
unsafe fn exec_environ(real: ExecFn, file: *const c_char, argv: *const *const c_char) -> i32 {
    if unsafe { path_is_nix(file) } {
        return unsafe { real(file, argv) };
    }
    let envp = unsafe { environ } as *const *const c_char;
    let Some((owned, ptrs)) = unsafe { filtered_envp(envp) } else {
        return unsafe { real(file, argv) };
    };
    let saved = unsafe { environ };
    unsafe { environ = ptrs.as_ptr() as *mut *mut c_char };
    let rc = unsafe { real(file, argv) };
    unsafe { environ = saved };
    rc
}

/// # Safety
///
/// Standard `execv` contract; must only be reached through LD_PRELOAD
/// interposition with valid libc argument pointers.
#[no_mangle]
pub unsafe extern "C" fn execv(path: *const c_char, argv: *const *const c_char) -> i32 {
    let real: ExecFn = real!(c"execv".as_ptr(), ExecFn);
    unsafe { exec_environ(real, path, argv) }
}

/// # Safety
///
/// Standard `execvp` contract; must only be reached through LD_PRELOAD
/// interposition with valid libc argument pointers.
#[no_mangle]
pub unsafe extern "C" fn execvp(file: *const c_char, argv: *const *const c_char) -> i32 {
    let real: ExecFn = real!(c"execvp".as_ptr(), ExecFn);
    unsafe { exec_environ(real, file, argv) }
}

// execl / execlp: implemented in src/execl_shim.c (C-variadic definitions
// are nightly-only in Rust) and routed through our execv/execvp above.
