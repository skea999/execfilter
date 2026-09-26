# execfilter

LD_PRELOAD shim (Rust) that strips nixGL/glibc-only environment variables
from the environment of **non-nix** child processes.

License: ISC (see `LICENSE`).

## The problem

DMS (quickshell) runs from nix as a **glibc** binary, wrapped by nixGL.
nixGL exports `LD_LIBRARY_PATH` (nix mesa/libglvnd) plus other GL-related
variables, and **every app launched from the DMS menu inherits them**. A
musl (Alpine) app that loads EGL/GL at startup — GTK4 apps like nautilus,
simple-scan — picks up the nix *glibc* libGL/libEGL and dies:

```
Error relocating /nix/store/.../libGL.so.1: __memcpy_chk: symbol not found
```

nixGL cannot scope its variables to a single process, so the shim is
preloaded into the nix/glibc process tree: it hooks the exec family and
removes the harmful variables for children that are **not** nix binaries.
Children under `/nix/store` or `~/.nix-profile` keep the environment, so
nix apps keep their GL stack / `LD_LIBRARY_PATH`.

## Stripped variables

`LD_LIBRARY_PATH`, `LD_PRELOAD`, `__EGL_VENDOR_LIBRARY_FILENAMES`,
`GBM_BACKENDS_PATH`, `LIBGL_DRIVERS_PATH`, `LIBVA_DRIVERS_PATH`

## Hooked functions

`execve`, `execv`, `execvp`, `execvpe`, `posix_spawn`, `posix_spawnp` are
implemented in Rust (`src/lib.rs`). `execl`/`execlp` live in a tiny C TU
(`src/execl_shim.c`, compiled via `cc` in `build.rs`) because C-variadic
definitions are nightly-only in Rust — they route through our own
`execv`/`execvp`. All 8 must be present: glibc implements `execl`/`execlp`
on top of the internal `__execve`, which bypasses an `execve`-only hook.

`execl`/`execlp` must be intercepted explicitly: glibc implements them on
top of the internal `__execve`, which bypasses an `execve`-only hook.

## Requirements

- **GLIBC shared object** — the shim is preloaded into the glibc nix
  process tree; a musl build will not load. (This is why it is
  distributed via nix/GitHub releases, not via the Alpine aports repo.)
- Must live on an **executable** filesystem: `/tmp` is often mounted
  `noexec`, and ld.so silently ignores a preload that fails to map.

## Build

```sh
# host with any glibc Linux toolchain
make            # = cargo build --release
make symbols    # verify all 8 hooks are exported
make test       # fmt + clippy + unit tests

# or via nix
nix build       # -> ./result/lib/libexecfilter.so
```

CI (GitHub Actions) builds x86_64 + aarch64 glibc `.so` artifacts on every
push and attaches them to `v*` tag releases.

## Install

```sh
install -Dm755 target/release/libexecfilter.so /usr/local/lib/execfilter.so
# or: nix profile install .#execfilter  (path via
#     $(nix profile list | awk ...)/lib/libexecfilter.so)
```

## Wire-up

niri session (`~/.config/niri/config.kdl`):

```kdl
spawn-at-startup "sh" "-c" "mkdir -p /var/log/user; exec env \
    LD_PRELOAD=/usr/local/lib/execfilter.so nixGLIntel dms run \
    >>/var/log/user/dms.log 2>&1"
```

Pick the nixGL wrapper for the GPU: `nixGLIntel` / `nixGLNvidia` /
`nixGLMesa`.

opencode wrapper (`/usr/local/bin/opencode`) — the shim also gives nix
binaries a clean environment for their own children:

```sh
export LD_LIBRARY_PATH=<gcc-lib store path>
export LD_PRELOAD=/usr/local/lib/execfilter.so
```

## Tests

```sh
make test      # fmt + clippy + unit tests (filter logic)
make symbols   # all 8 hooks exported in the release .so
```

Unit tests cover the safe core (`is_nix`, `filter_env_vars`): store/profile
detection, symlink resolution, exact-key prefix matching, full strip set.
The CI additionally asserts the 8 exported hook symbols on both
x86_64 and aarch64 glibc builds.

## Note on musl vs glibc

A musl build of this shim would be useless (it would be preloaded into
glibc processes). On Alpine, consume it via nix or the GitHub release
artifact — not via aports.
