$ErrorActionPreference = "Stop"
cargo xtask check @args
exit $LASTEXITCODE
