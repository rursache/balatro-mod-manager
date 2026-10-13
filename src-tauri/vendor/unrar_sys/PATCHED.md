# Patched unrar_sys 0.5.8

Local copy of the crates.io release, used through `[patch.crates-io]` in the workspace `Cargo.toml`, so the Windows build can be cross compiled from macOS/Linux with cargo-xwin

Changes from upstream:
- `build.rs` decides libraries and sources from the target (`CARGO_CFG_TARGET_OS`/`CARGO_CFG_TARGET_ENV`) instead of the host, compiles `isnt.cpp` for Windows and links `advapi32` (same as https://github.com/muja/unrar.rs/pull/81)
- `vendor/unrar/blake2s_sse.cpp`, `rijndael.hpp`, `rs16.hpp`: the per function SSE/AES target attributes are also enabled for clang-cl (`__clang__`), which does not define `__GNUC__`. UnRAR still picks these code paths at runtime via cpuid

Drop this copy once an upstream release includes both fixes
