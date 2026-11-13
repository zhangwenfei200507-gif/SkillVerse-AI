# SQLx SQLite compatibility patch

`sqlx-sqlite` is the crates.io 0.8.6 release from SQLx revision `bab1b022bd56a64f9a08b46b36b97c5cff19d77e`. The original `.crate` archive SHA-256 is `c2d12fe70b2c1b4401038055f90f151b78208de1f9f89a7dbfd41587a10c3eea`. Both upstream licenses and the original manifest are retained.

The single dependency change in the published manifest raises `libsqlite3-sys` from `0.30.1` to exactly `0.39.1` for RUSTSEC-2026-0459. Driver sources retain their published bytes. The application uses ordinary bundled SQLite 3.54.1. The vendor patch can be removed once a compatible upstream SQLx release resolves a patched bundled SQLite version and passes the Windows migration and interruption tests.

[RustSec advisory](https://rustsec.org/advisories/RUSTSEC-2026-0459.html) · [SQLx upstream dependency discussion](https://github.com/transact-rs/sqlx/issues/4379)
