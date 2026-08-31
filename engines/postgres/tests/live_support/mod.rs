use std::env;
use std::sync::atomic::{AtomicU64, Ordering};

use native_tls::{Certificate, TlsConnector};
use postgres::config::{SslMode, SslNegotiation};
use postgres::{Client, Config};
use postgres_native_tls::{MakeTlsConnector, set_postgresql_alpn};

use dol_postgres::PostgresRuntimeConfig;

static NEXT_SCHEMA: AtomicU64 = AtomicU64::new(1);

pub struct LiveDb {
    url: String,
    schema: String,
    client: Client,
}

impl LiveDb {
    pub fn open() -> Self {
        let url = env::var("DOL_POSTGRES_TEST_URL")
            .expect("DOL_POSTGRES_TEST_URL must be set for the live PostgreSQL conformance gate");
        let mut client = connect(&url);
        let schema = format!(
            "dol_h2b_{}_{}",
            std::process::id(),
            NEXT_SCHEMA.fetch_add(1, Ordering::Relaxed)
        );
        client
            .batch_execute(&format!("CREATE SCHEMA {}", quote_identifier(&schema)))
            .expect("live test principal must be able to create an isolated schema");
        Self {
            url,
            schema,
            client,
        }
    }

    pub fn schema(&self) -> &str {
        &self.schema
    }

    pub fn runtime_config(&self) -> PostgresRuntimeConfig {
        let runtime = PostgresRuntimeConfig::parse(&self.url)
            .expect("live PostgreSQL runtime configuration must parse");
        match env::var_os("DOL_POSTGRES_TEST_CA") {
            Some(path) if !path.is_empty() => runtime
                .with_trusted_ca_file(path)
                .expect("DOL_POSTGRES_TEST_CA must contain a valid PEM CA certificate"),
            _ => runtime,
        }
    }

    pub fn client_mut(&mut self) -> &mut Client {
        &mut self.client
    }

    pub fn batch(&mut self, sql: &str) {
        self.client
            .batch_execute(sql)
            .expect("live PostgreSQL fixture SQL must succeed");
    }

    pub fn qualified(&self, table: &str) -> String {
        format!(
            "{}.{}",
            quote_identifier(&self.schema),
            quote_identifier(table)
        )
    }
}

impl Drop for LiveDb {
    fn drop(&mut self) {
        let _ = self.client.batch_execute(&format!(
            "DROP SCHEMA IF EXISTS {} CASCADE",
            quote_identifier(&self.schema)
        ));
    }
}

fn connect(url: &str) -> Client {
    let config = url
        .parse::<Config>()
        .expect("DOL_POSTGRES_TEST_URL must be a valid PostgreSQL connection configuration");
    assert_eq!(
        config.get_ssl_mode(),
        SslMode::Require,
        "DOL_POSTGRES_TEST_URL must use sslmode=require; plaintext fallback is not permitted",
    );
    let mut builder = TlsConnector::builder();
    if let Some(path) = env::var_os("DOL_POSTGRES_TEST_CA").filter(|path| !path.is_empty()) {
        let pem = std::fs::read(path).expect("DOL_POSTGRES_TEST_CA must be readable");
        let certificates = Certificate::stack_from_pem(&pem)
            .expect("DOL_POSTGRES_TEST_CA must contain a valid PEM CA certificate bundle");
        assert!(
            !certificates.is_empty(),
            "DOL_POSTGRES_TEST_CA must contain at least one PEM CA certificate"
        );
        for certificate in certificates {
            builder.add_root_certificate(certificate);
        }
    }
    if config.get_ssl_negotiation() == SslNegotiation::Direct {
        set_postgresql_alpn(&mut builder);
    }
    let tls = builder
        .build()
        .map(MakeTlsConnector::new)
        .expect("platform TLS connector must initialize");
    config
        .connect(tls)
        .expect("live PostgreSQL test connection must succeed over authenticated TLS")
}

fn quote_identifier(value: &str) -> String {
    assert!(
        value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_'),
        "live-test identifiers are generated internally and must remain conservative"
    );
    format!("\"{value}\"")
}
