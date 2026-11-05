# Quality checks

`make quality` runs formatting, strict Clippy, C smoke-source analysis,
Doxygen, and warning-free Rustdoc. `make test` runs Rust and C ABI tests.
`make coverage` checks production Rust line and branch coverage on Debian 13
amd64. `make platform-verify` checks staged installation, Debian packages, and
the source archive on each native Debian 13 architecture.

The required GitHub pull request gate runs those checks through the shared
workflow. Run local targeted checks in the project-owned labeled container
using `tools/run-in-quality-container.sh` and a freshly pulled quality image.
