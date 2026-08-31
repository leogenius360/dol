#!/bin/sh
set -eu

cat > "$PGDATA/pg_hba.conf" <<'HBA'
local   all             all                                     trust
hostnossl all           all             0.0.0.0/0               reject
hostnossl all           all             ::/0                    reject
hostssl dol             dol_test        0.0.0.0/0               scram-sha-256
hostssl dol             dol_test        ::/0                    scram-sha-256
hostssl all             all             0.0.0.0/0               reject
hostssl all             all             ::/0                    reject
HBA

chown postgres:postgres "$PGDATA/pg_hba.conf"
chmod 0600 "$PGDATA/pg_hba.conf"
