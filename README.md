# Shared remote-base control library

The `librptadvremote.so.1` contract provides portable radio configuration,
capability validation, and configuration-only named memories. One non-audio
physical owner serializes calls to each opaque context. This library owns no
Asterisk channel, audio stream, CAT port, or GPIO device.

The public descriptor is declared in `include/rptadvremote/rptadvremote.h`.
Setup copies its borrowed spans and validates the complete safety policy
before returning an owned handle. A configured-memory read describes desired
configuration; it does not report physical state or authorize RF transmit.

Use `make test` for Rust tests and the installed-descriptor C smoke test.
`make lint static-analysis docs` checks the changed developer-facing surface.
The maintained GitHub workflow runs the complete native platform gate before
a pull request may merge. This component is not yet released.
