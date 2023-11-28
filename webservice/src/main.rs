#![deny(
    clippy::panic,
    clippy::expect_used,
    clippy::exit,
    clippy::todo,
    clippy::unwrap_in_result
)]

#[macro_use]
extern crate log;
#[macro_use]
extern crate serde_derive;
#[macro_use]
extern crate diesel;

use ::r2d2::Pool;
use actix_cors::Cors;
use actix_web::body::MessageBody;
use actix_web::dev::{ServiceFactory, ServiceRequest, ServiceResponse};
use actix_web::{http, middleware::Logger, web, App, HttpRequest, HttpResponse, HttpServer};
use administration::BackgroundJobs;
use anyhow::bail;
use api::administration;
use clap::Arg;
use diesel::prelude::*;
use diesel::r2d2::{self, ConnectionManager};
use diesel_migrations::{embed_migrations, EmbeddedMigrations, MigrationHarness};
use graphannis::CorpusStorage;
use settings::Settings;
use simplelog::{LevelFilter, SimpleLogger, TermLogger};
use std::{
    io::{Error, ErrorKind, Result},
    path::PathBuf,
};

mod actions;
mod api;
mod auth;

mod errors;
mod extractors;
mod models;
mod schema;
mod settings;

#[cfg(not(target_env = "msvc"))]
use tikv_jemallocator::Jemalloc;

#[cfg(not(target_env = "msvc"))]
#[global_allocator]
static GLOBAL: Jemalloc = Jemalloc;

const API_VERSION: &str = "/v1";

type DbPool = r2d2::Pool<ConnectionManager<SqliteConnection>>;

pub const MIGRATIONS: EmbeddedMigrations = embed_migrations!("migrations");

fn init_app_state() -> anyhow::Result<(graphannis::CorpusStorage, settings::Settings, DbPool)> {
    // Parse CLI arguments
    let matches = clap::App::new("graphANNIS web service")
        .version(env!("CARGO_PKG_VERSION"))
        .author(env!("CARGO_PKG_AUTHORS"))
        .about("Web service line interface to graphANNIS.")
        .arg(
            Arg::with_name("config")
                .short("c")
                .long("config")
                .help("Configuration file location")
                .takes_value(true),
        )
        .get_matches();

    // Load configuration file(s)
    let settings = settings::Settings::with_file(matches.value_of_lossy("config"))?;

    let log_filter = if settings.logging.debug {
        LevelFilter::Debug
    } else {
        LevelFilter::Info
    };

    let mut log_config = simplelog::ConfigBuilder::new();
    log_config.add_filter_ignore_str("rustyline:");
    if settings.logging.debug {
        warn!("Enabling request logging to console in debug mode");
    } else {
        log_config.add_filter_ignore_str("actix_web:");
    }

    let log_config = log_config.build();

    if let Err(e) = TermLogger::init(
        log_filter,
        log_config.clone(),
        simplelog::TerminalMode::Mixed,
        simplelog::ColorChoice::Auto,
    ) {
        println!("Error, can't initialize the terminal log output: {}.\nWill degrade to a more simple logger", e);
        if let Err(e_simple) = SimpleLogger::init(log_filter, log_config) {
            println!("Simple logging failed too: {}", e_simple);
        }
    }

    info!("Logging with level {}", log_filter);

    // Create a graphANNIS corpus storage as shared state
    let data_dir = std::path::PathBuf::from(&settings.database.graphannis);
    let cs = graphannis::CorpusStorage::with_cache_strategy(
        &data_dir,
        settings.database.cache.clone(),
        true,
    )?;

    // Add a connection pool to the SQLite database

    let manager = ConnectionManager::<SqliteConnection>::new(&settings.database.sqlite);
    let db_pool = r2d2::Pool::builder().build(manager)?;

    // Make sure the database has all migrations applied
    let mut conn = db_pool.get()?;
    if let Err(e) = conn.run_pending_migrations(MIGRATIONS) {
        bail!("Database migration failed: {e}");
    }

    info!(
        "Using database {} with at most {} of RAM for the corpus cache.",
        PathBuf::from(&settings.database.sqlite)
            .canonicalize()?
            .to_string_lossy(),
        &settings.database.cache
    );
    if let Some(timeout) = &settings.database.query_timeout {
        info!("Queries timeout set to {} seconds", timeout);
    }

    Ok((cs, settings, db_pool))
}

fn create_app(
    cs: web::Data<CorpusStorage>,
    settings: web::Data<Settings>,
    db_pool: web::Data<Pool<ConnectionManager<SqliteConnection>>>,
) -> App<
    impl ServiceFactory<
        ServiceRequest,
        Response = ServiceResponse<impl MessageBody>,
        Config = (),
        InitError = (),
        Error = actix_web::Error,
    >,
> {
    let logger = if settings.logging.debug {
        // Log all requests in debug
        Logger::default()
    } else {
        Logger::default().exclude_regex(".*")
    };

    // Create a list of background jobs behind a Mutex
    let background_jobs = web::Data::new(BackgroundJobs::default());

    App::new()
        .wrap(
            Cors::default()
                .allowed_headers(vec![http::header::AUTHORIZATION, http::header::ACCEPT])
                .allowed_header(http::header::CONTENT_TYPE),
        )
        .app_data(cs)
        .app_data(settings)
        .app_data(db_pool)
        .app_data(background_jobs)
        .wrap(logger)
        .service(
            web::scope(API_VERSION)
                .route("openapi.yml", web::get().to(get_api_spec))
                .route(
                    "/import",
                    web::post().to(api::administration::import_corpus),
                )
                .route(
                    "/export",
                    web::post().to(api::administration::export_corpus),
                )
                .route("/jobs/{uuid}", web::get().to(api::administration::jobs))
                .service(
                    web::scope("/search")
                        .route("/count", web::post().to(api::search::count))
                        .route("/find", web::post().to(api::search::find))
                        .route("/frequency", web::post().to(api::search::frequency))
                        .route(
                            "/node-descriptions",
                            web::get().to(api::search::node_descriptions),
                        ),
                )
                .service(
                    web::scope("/corpora")
                        .route("", web::get().to(api::corpora::list))
                        .route("/{corpus}", web::delete().to(api::corpora::delete))
                        .route(
                            "/{corpus}/configuration",
                            web::get().to(api::corpora::configuration),
                        )
                        .route(
                            "/{corpus}/node-annotations",
                            web::get().to(api::corpora::node_annotations),
                        )
                        .route(
                            "/{corpus}/components",
                            web::get().to(api::corpora::list_components),
                        )
                        .route(
                            "/{corpus}/edge-annotations/{type}/{layer}/{name}/",
                            web::get().to(api::corpora::edge_annotations),
                        )
                        .route("/{corpus}/subgraph", web::post().to(api::corpora::subgraph))
                        .route(
                            "/{corpus}/subgraph-for-query",
                            web::get().to(api::corpora::subgraph_for_query),
                        )
                        .route(
                            "/{corpus}/files/{name}",
                            web::get().to(api::corpora::file_content),
                        )
                        .route("/{corpus}/files", web::get().to(api::corpora::list_files)),
                )
                .service(
                    web::scope("/groups")
                        .route("", web::get().to(administration::list_groups))
                        .route("/{name}", web::delete().to(administration::delete_group))
                        .route("/{name}", web::put().to(administration::put_group)),
                ),
        )
}

async fn get_api_spec(_req: HttpRequest) -> HttpResponse {
    HttpResponse::Ok()
        .content_type("application/x-yaml")
        .body(include_str!("openapi.yml"))
}

#[actix_web::main]
async fn main() -> Result<()> {
    // Initialize application and its state
    let (cs, settings, db_pool) = init_app_state().map_err(|e| {
        Error::new(
            ErrorKind::Other,
            format!("Could not initialize graphANNIS service: {:?}", e),
        )
    })?;

    let bind_address = format!("{}:{}", &settings.bind.host, &settings.bind.port);
    let cs = web::Data::new(cs);
    let settings = web::Data::new(settings);
    let db_pool = web::Data::new(db_pool);

    // Run server
    HttpServer::new(move || create_app(cs.clone(), settings.clone(), db_pool.clone()))
        .bind(bind_address)?
        .run()
        .await
}

#[cfg(test)]
pub mod tests {
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use actix_web::{
        body::MessageBody,
        dev::{ServiceFactory, ServiceRequest, ServiceResponse},
        web, App,
    };
    use diesel::{r2d2::ConnectionManager, SqliteConnection};
    use diesel_migrations::MigrationHarness;
    use graphannis::CorpusStorage;
    use jsonwebtoken::EncodingKey;

    use crate::{
        auth::Claims,
        settings::{JWTVerification, Settings},
    };

    pub const JWT_SECRET: &str = "not-a-secret";

    pub fn create_empty_dbpool() -> r2d2::Pool<ConnectionManager<SqliteConnection>> {
        let manager = ConnectionManager::<SqliteConnection>::new(":memory:");
        let db_pool = r2d2::Pool::builder().build(manager).unwrap();
        let mut conn = db_pool.get().unwrap();
        conn.run_pending_migrations(crate::MIGRATIONS).unwrap();

        db_pool
    }

    pub fn create_test_app(
        cs: web::Data<CorpusStorage>,
    ) -> App<
        impl ServiceFactory<
            ServiceRequest,
            Response = ServiceResponse<impl MessageBody>,
            Config = (),
            InitError = (),
            Error = actix_web::Error,
        >,
    > {
        // Create an app that uses a string as secret so we can sign our own JWT
        // token.
        let mut settings = Settings::default();
        settings.auth.token_verification = JWTVerification::HS256 {
            secret: JWT_SECRET.to_string(),
        };

        let db_pool = create_empty_dbpool();

        let settings = web::Data::new(settings);
        let db_pool = web::Data::new(db_pool);

        let app = crate::create_app(cs, settings, db_pool);
        app
    }

    pub fn create_auth_header() -> (&'static str, String) {
        // Create an auth header for an admin
        let in_sixty_minutes = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .checked_add(Duration::from_secs(3600))
            .unwrap();
        let admin_claims = Claims {
            sub: "admin".to_string(),
            exp: Some(in_sixty_minutes.as_millis() as i64),
            roles: vec!["admin".to_string()],
            groups: vec![],
        };
        let bearer_token = jsonwebtoken::encode(
            &jsonwebtoken::Header::default(),
            &admin_claims,
            &EncodingKey::from_secret(JWT_SECRET.as_ref()),
        )
        .unwrap();
        ("Authorization", format!("Bearer {bearer_token}"))
    }
}
