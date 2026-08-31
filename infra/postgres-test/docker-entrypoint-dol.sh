#!/bin/sh
set -eu

TLS_DIR=/var/lib/dol-postgres-test/tls
mkdir -p "$TLS_DIR"

if [ ! -s "$TLS_DIR/ca.crt" ] || [ ! -s "$TLS_DIR/server.crt" ] || [ ! -s "$TLS_DIR/server.key" ]; then
    rm -f "$TLS_DIR"/ca.* "$TLS_DIR"/server.*

    openssl req -x509 -newkey rsa:2048 -nodes -sha256 \
        -keyout "$TLS_DIR/ca.key" \
        -out "$TLS_DIR/ca.crt" \
        -days 30 \
        -subj "/CN=DOL PostgreSQL Test CA" \
        -addext "basicConstraints=critical,CA:TRUE" \
        -addext "keyUsage=critical,keyCertSign,cRLSign"

    openssl req -new -newkey rsa:2048 -nodes -sha256 \
        -keyout "$TLS_DIR/server.key" \
        -out "$TLS_DIR/server.csr" \
        -subj "/CN=localhost"

    cat > "$TLS_DIR/server.ext" <<'EXT'
subjectAltName=DNS:localhost,IP:127.0.0.1
basicConstraints=critical,CA:FALSE
keyUsage=critical,digitalSignature,keyEncipherment
extendedKeyUsage=serverAuth
EXT

    openssl x509 -req -sha256 \
        -in "$TLS_DIR/server.csr" \
        -CA "$TLS_DIR/ca.crt" \
        -CAkey "$TLS_DIR/ca.key" \
        -CAcreateserial \
        -out "$TLS_DIR/server.crt" \
        -days 30 \
        -extfile "$TLS_DIR/server.ext"

    rm -f "$TLS_DIR/server.csr" "$TLS_DIR/server.ext" "$TLS_DIR/ca.srl"
fi

chown -R postgres:postgres "$TLS_DIR"
chmod 0700 "$TLS_DIR"
chmod 0600 "$TLS_DIR/ca.key" "$TLS_DIR/server.key"
chmod 0644 "$TLS_DIR/ca.crt" "$TLS_DIR/server.crt"

if [ -s "$PGDATA/PG_VERSION" ]; then
    /usr/local/bin/dol-postgres-test-hba
fi

if [ "${1:-}" = "postgres" ]; then
    set -- "$@" \
        -c ssl=on \
        -c ssl_cert_file="$TLS_DIR/server.crt" \
        -c ssl_key_file="$TLS_DIR/server.key" \
        -c ssl_min_protocol_version=TLSv1.2
fi

exec /usr/local/bin/docker-entrypoint.sh "$@"
