FROM postgres:18.6-bookworm@sha256:1c59e2c3c818eaa0f0628f695b36e7c9e362d6b219b36a54a32df645cbd7e1af

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates openssl \
    && rm -rf /var/lib/apt/lists/*

COPY docker-entrypoint-dol.sh /usr/local/bin/dol-postgres-test-entrypoint
COPY enforce-hba.sh /usr/local/bin/dol-postgres-test-hba
COPY init/ /docker-entrypoint-initdb.d/

RUN chmod 0755 /usr/local/bin/dol-postgres-test-entrypoint \
    /usr/local/bin/dol-postgres-test-hba \
    /docker-entrypoint-initdb.d/99-enforce-tls.sh

ENTRYPOINT ["/usr/local/bin/dol-postgres-test-entrypoint"]
CMD ["postgres"]
