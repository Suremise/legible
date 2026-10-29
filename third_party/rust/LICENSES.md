# Rust crates outside the permissive list

Every Rust crate Legible ships is MIT or equally permissive (`deny.toml`), except these two, which
the maintainer allowed on 27 Oct 2026 (docs/ARCHITECTURE.md, Licences). THIRD-PARTY-NOTICES must
credit them as below, with the MPL-2.0 text in [MPL-2.0.txt](MPL-2.0.txt).

| Crate | Version | Licence | Why it's in | Source |
|---|---|---|---|---|
| `cssparser` | 0.38.0 | MPL-2.0 | a dependency of `ammonia` (the HTML sanitiser) from 4.1 on, for its CSS filtering, which Legible doesn't use | <https://static.crates.io/crates/cssparser/cssparser-0.38.0.crate> (SHA-256 `11119743ad110e8c1bdccd930d7f5c30c99e5fc76a7b63ec9807e84eef0c5f59`); repository <https://github.com/servo/rust-cssparser> |
| `dtoa-short` | 0.3.5 | MPL-2.0 | a dependency of `cssparser` | <https://static.crates.io/crates/dtoa-short/dtoa-short-0.3.5.crate> (SHA-256 `cd1511a7b6a56299bd043a9c167a6d2bfb37bf84a6dfceaba651168adfb43c87`); repository <https://github.com/upsuper/dtoa-short> |

**The terms:** MPL-2.0 is file-level copyleft. Legible ships both crates **unmodified**, compiled
into the exe; their source is available at the addresses above (the SHA-256s are the crates'
checksums in `Cargo.lock`). If either is ever modified, its changed files must be published under
MPL-2.0.

**Keeping it narrow:** `deny.toml` allows MPL-2.0 for these two crates at exactly these versions.
A new version, or any other MPL crate, fails `cargo deny check` until it's reviewed and this
file is updated.
