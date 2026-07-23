# Fuselect

Fuselect is a local, privacy-first Fusion gateway for Coding Agents. It will let
Codex use one local endpoint while a bounded panel of cloud Workers can advise a
single tool-capable outer Worker.

This repository is at the foundation stage. The command skeleton is intentional;
the public protocol, local storage, Codex setup, and Fusion engine are implemented
incrementally with compatibility and privacy tests.

## Status

The current milestone establishes the Rust CLI baseline. It is not yet a usable
gateway and must not be used for production traffic.

## Development prerequisites

- Rust stable (MSRV: 1.85).
- Windows: the Rust MSVC toolchain plus Visual Studio Build Tools with the
  **Desktop development with C++** workload. VS Code alone does not include the
  required `link.exe` linker.
- Linux: a standard C/C++ build toolchain for the selected Rust target.

Run the local checks with:

```powershell
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --all-targets
```

## License

Licensed under [Apache-2.0](LICENSE).
