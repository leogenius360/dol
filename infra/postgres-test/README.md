# Managed PostgreSQL conformance fixture

This directory owns DOL's reproducible live PostgreSQL test environment. Do not use its credentials or certificates outside local/CI conformance testing.

Use the repository tasks rather than invoking Compose directly:

```text
cargo xtask postgres-up
cargo xtask postgres-live
cargo xtask postgres-down
```

The fixture is loopback-only, digest-pinned to PostgreSQL 18.6, uses SCRAM authentication, rejects non-TLS TCP connections, and permits remote TCP login only for the `dol_test` role against the `dol` database. The role may connect and create isolated schemas but is not the bootstrap administrator.

TLS material is generated when the container starts. The CA private key and server private key remain in a Docker volume. `postgres-up` exports only the CA certificate to ignored `.dol/postgres-test/ca.crt` so the Rust runtime can authenticate the localhost server without disabling hostname or certificate verification.

Set `DOL_POSTGRES_TEST_PORT` before `postgres-up` to override the default loopback port `55432`. Set `DOL_POSTGRES_TEST_URL` to bypass the managed fixture and run against an external TLS PostgreSQL instance; use `DOL_POSTGRES_TEST_CA` when that instance is signed by a private CA.
