#![warn(rust_2018_idioms)]
#![forbid(unsafe_code)]

#[macro_use]
extern crate slog_scope;

use std::collections::HashMap;
use std::sync::Arc;
use std::{env, vec::Vec};

use actix_web::HttpServer;
use docopt::Docopt;
use serde::Deserialize;
use std::sync::RwLock;

use autoconnect_settings::{AppState, Settings};
use autoconnect_web::{build_app, client::ClientChannels, config};
use autopush_common::errors::{ApcErrorKind, Result};

mod middleware;

pub type LocalError = autopush_common::errors::ApcError;

const USAGE: &str = "
Usage: autopush_rs [options]

Options:
    -h, --help                          Show this message.
    --config-connection=CONFIGFILE      Connection configuration file path.
    --config-shared=CONFIGFILE          Common configuration file path.
";

#[derive(Debug, Deserialize)]
struct Args {
    flag_config_connection: Option<String>,
    flag_config_shared: Option<String>,
}

#[actix_web::main]
async fn main() -> Result<()> {
    env_logger::init();

    let args: Args = Docopt::new(USAGE)
        .and_then(|d| d.deserialize())
        .unwrap_or_else(|e| e.exit());
    let mut filenames = Vec::new();
    if let Some(shared_filename) = args.flag_config_shared {
        filenames.push(shared_filename);
    }
    if let Some(config_filename) = args.flag_config_connection {
        filenames.push(config_filename);
    }
    let settings =
        Settings::with_env_and_config_files(&filenames).map_err(ApcErrorKind::ConfigError)?;

    //TODO: Eventually this will match between the various storage engines that
    // we support. For now, it's just the one, DynamoDB.
    // Perform any app global storage initialization.
    match autopush_common::db::StorageType::from_dsn(&settings.db_dsn) {
        autopush_common::db::StorageType::DynamoDb => {
            env::set_var("AWS_LOCAL_DYNAMODB", settings.db_dsn.clone().unwrap())
        }
        autopush_common::db::StorageType::INVALID => {
            panic!("Invalid Storage type. Check DB_DSN.");
        }
    }

    // Sentry requires the environment variable "SENTRY_DSN".
    if env::var("SENTRY_DSN")
        .unwrap_or_else(|_| "".to_owned())
        .is_empty()
    {
        print!("SENTRY_DSN not set. Logging disabled.");
    }

    let _guard = sentry::init(sentry::ClientOptions {
        release: sentry::release_name!(),
        session_mode: sentry::SessionMode::Request, // new session per request
        auto_session_tracking: true,
        ..Default::default()
    });

    let port = settings.port;
    let app_state = AppState::from_settings(settings)?;
    let _client_channels: ClientChannels = Arc::new(RwLock::new(HashMap::new()));

    info!("Starting autoconnect on port {:?}", port);
    let srv = HttpServer::new(move || {
        let app = build_app!(app_state);
        // TODO: should live in build_app!
        app.wrap(crate::middleware::sentry::SentryWrapper::new(
            app_state.metrics.clone(),
            "error".to_owned(),
        ))
    })
    .bind(("0.0.0.0", port))?
    .run();

    info!("Server starting, port: {}", port);
    srv.await.map_err(|e| e.into()).map(|v| {
        info!("Shutting down autoconnect");
        v
    })
}