//! `cairn-server` — the small central server behind shared project memory.
//!
//! It stores only the allowlist in FR-055. There is no observations table and
//! no route that would accept observation content.

mod api;
mod auth;
mod commands;
mod db;
mod error;
mod events;
mod global;
mod sync;
mod version;

use axum::http::{header, Method};
use axum::Router;
use clap::{Parser, Subcommand};
use sqlx::PgPool;
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;
use uuid::Uuid;

#[derive(Clone)]
pub struct AppState {
    pub pool: PgPool,
    /// Shared knowledge of the newest published release.
    pub releases: version::ReleaseCache,
    /// Whether the session cookie may only travel over HTTPS.
    pub secure_cookies: bool,
    /// The highest migration this database has applied (FR-415).
    pub schema_version: i64,
    /// This server's own identity, established once and never reassigned
    /// (FR-415). A local store pins its team knowledge to it, so it has to be
    /// the same value across restarts — which is why it is read from the
    /// database rather than generated here.
    ///
    /// `None` below schema 3: the table arrives with migration 3, and a
    /// deployment held back for a staged rollout has no instance identity yet.
    /// A client sees the field absent and correctly concludes this server cannot
    /// hold team knowledge — which is the same conclusion the capability list
    /// gives it.
    pub server_instance_id: Option<Uuid>,
    /// The account named by `CAIRN_ADMIN_EMAIL`, if any.
    ///
    /// Carried on the state because two routes must refuse to touch it, and
    /// both need to know which account it is. `None` when the operator seeded
    /// nothing — in which case no account is exempt.
    pub environment_account: Option<String>,
}

impl AppState {
    /// Is this the account the environment defines?
    ///
    /// Compared on the same normalized form `ensure_admin` upserts by, so the
    /// two never disagree about which row they mean.
    pub fn is_environment_account(&self, email: &str) -> bool {
        self.environment_account
            .as_deref()
            .is_some_and(|configured| configured == email.trim().to_lowercase())
    }

    /// The `Secure` attribute, or nothing.
    ///
    /// Marking the cookie `Secure` on a plain-HTTP deployment would make the
    /// browser drop it and nobody could sign in, so this follows the origin the
    /// site is actually served from rather than being hardcoded.
    pub fn cookie_secure_attr(&self) -> &'static str {
        if self.secure_cookies {
            "; Secure"
        } else {
            ""
        }
    }
}

#[derive(Parser, Debug)]
#[command(name = "cairn-server", about = "Cairn shared memory server", version)]
struct Args {
    /// PostgreSQL connection string.
    // `global` so it can follow a subcommand as well as precede it. A caller
    // reaching for `cairn-server users add --database-url ...` is writing the
    // obvious thing, and clap would otherwise refuse it.
    #[arg(
        long,
        global = true,
        env = "DATABASE_URL",
        default_value = "postgres://cairn:cairn@localhost:5433/cairn"
    )]
    database_url: String,
    /// Address to listen on.
    #[arg(long, env = "CAIRN_SERVER_ADDR", default_value = "127.0.0.1:8080")]
    addr: String,
    /// Origin allowed to call the API from a browser (the web UI).
    ///
    /// Also decides whether the session cookie is marked `Secure`: an `https`
    /// origin means the cookie must never travel in clear.
    #[arg(long, env = "CAIRN_WEB_ORIGIN")]
    web_origin: Option<String>,
    /// Size of the PostgreSQL connection pool.
    ///
    /// The default suits a deployment where this server owns its database. The
    /// end-to-end suite runs many servers at once against one PostgreSQL and
    /// asks each for a small share, so they do not exhaust it between them.
    #[arg(long, env = "CAIRN_SERVER_MAX_CONNECTIONS", default_value_t = db::DEFAULT_MAX_CONNECTIONS)]
    max_connections: u32,
    /// Email of the account defined by the environment.
    ///
    /// Set it with `--admin-password` and the server creates that account on
    /// start, so a fresh deployment has someone able to sign in. Leave both
    /// unset and the server seeds nothing.
    #[arg(long, env = "CAIRN_ADMIN_EMAIL")]
    admin_email: Option<String>,
    /// Password for the environment-defined account. At least 8 characters.
    ///
    /// Re-applied on every start: this variable is the account's password, not
    /// merely its initial one.
    #[arg(long, env = "CAIRN_ADMIN_PASSWORD")]
    admin_password: Option<String>,
    /// Display name for the environment-defined account.
    #[arg(long, env = "CAIRN_ADMIN_DISPLAY_NAME", default_value = "Admin")]
    admin_display_name: String,
    /// Highest migration to apply, for a staged rollout.
    ///
    /// The binary ships first and the schema moves when the operator is ready.
    /// A held-back deployment advertises the capabilities of the schema it
    /// actually applied, so a peer queues only work this server can hold and
    /// retains the rest until the migration runs (FR-415).
    #[arg(long, env = "CAIRN_MAX_SCHEMA_VERSION", default_value_t = db::SCHEMA_VERSION)]
    max_schema_version: i64,
    /// Run an operator command instead of serving.
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Manage accounts. There is no route that creates one.
    #[command(subcommand)]
    Users(UserCommand),
}

#[derive(Subcommand, Debug)]
enum UserCommand {
    /// Create an account.
    ///
    /// This exists because `POST /api/auth/register` was removed: it was an
    /// unauthenticated route that let anyone who could reach the server create
    /// an account, which was the first step of a complete compromise chain.
    /// Creating accounts is an operator act, so it happens here — locally,
    /// against the database, by whoever already controls the host. That is the
    /// same trust boundary `--admin-email` already sits on.
    Add {
        /// Email address. Lowercased and trimmed.
        #[arg(long)]
        email: String,
        /// Human-readable name.
        #[arg(long)]
        display_name: String,
        /// At least 8 characters.
        #[arg(long, env = "CAIRN_NEW_USER_PASSWORD")]
        password: String,
        /// Require the account to change this password before doing anything
        /// else.
        ///
        /// Off by default here, and on by default for `POST /api/admin/users`.
        /// The two differ because the operator's intent differs: this
        /// subcommand's caller *chose* the password and typed it, which is how
        /// scripted provisioning works, and forcing a change would break it. The
        /// HTTP route generates a password the new account has never seen, and
        /// there a forced change is the whole point.
        #[arg(long)]
        must_change_password: bool,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    init_tracing();

    // Migrations run on start, so a fresh deployment needs no separate step.
    // The admin email reaches the migrations as well as `seed_admin` below:
    // migration 3's role backfill needs to know which existing account the
    // environment names before it picks one (FR-414, FR-524).
    let pool = db::connect(
        &args.database_url,
        args.max_connections,
        args.max_schema_version,
        args.admin_email.as_deref(),
    )
    .await?;

    // An operator command runs against the schema and exits. It deliberately
    // happens after `db::connect` — so the migration state is the same one the
    // server would see — and before `seed_admin`, so creating a user never
    // depends on the admin variables being set.
    if let Some(command) = &args.command {
        return run_command(&pool, command).await;
    }

    seed_admin(&pool, &args).await?;

    // What the database actually holds, not what this binary could apply. A
    // held-back deployment must advertise the smaller answer.
    let schema_version = db::applied_version(&pool).await?;
    if schema_version < db::SCHEMA_VERSION {
        tracing::warn!(
            schema_version,
            binary_supports = db::SCHEMA_VERSION,
            "running below this build's schema; peers will retain work this server cannot hold"
        );
    }

    let secure_cookies = args
        .web_origin
        .as_deref()
        .is_some_and(|origin| origin.starts_with("https://"));
    if !secure_cookies {
        tracing::warn!(
            "session cookies are not marked Secure; set CAIRN_WEB_ORIGIN to an https origin"
        );
    }
    // Read, not generated: migration 3 inserts exactly one row and nothing ever
    // replaces it, so this value survives every restart. Generating it here
    // would mint a new identity on each start and every linked store's team
    // knowledge would look like it came from a different server.
    //
    // Absent below schema 3, because the table arrives with migration 3 — and a
    // deployment held back for a staged rollout is a supported configuration, not
    // an error. Reading it unconditionally made every schema-pinned server fail
    // to start with `relation "server_instance" does not exist`.
    let server_instance_id: Option<Uuid> = if schema_version >= 3 {
        sqlx::query_scalar("SELECT id FROM server_instance LIMIT 1")
            .fetch_optional(&pool)
            .await?
    } else {
        None
    };

    let state = AppState {
        pool,
        secure_cookies,
        releases: version::ReleaseCache::new(),
        schema_version,
        server_instance_id,
        environment_account: args
            .admin_email
            .as_deref()
            .map(|e| e.trim().to_lowercase())
            .filter(|e| !e.is_empty()),
    };

    // A credentialed browser request cannot be paired with wildcard headers or
    // methods, so the web-origin branch names both explicitly.
    let cors = match &args.web_origin {
        Some(origin) => CorsLayer::new()
            .allow_origin(origin.parse::<axum::http::HeaderValue>()?)
            .allow_credentials(true)
            .allow_headers([header::CONTENT_TYPE, header::AUTHORIZATION, header::COOKIE])
            .allow_methods([Method::GET, Method::POST, Method::DELETE, Method::OPTIONS]),
        None => CorsLayer::new()
            .allow_origin(Any)
            .allow_headers(Any)
            .allow_methods(Any),
    };

    let app: Router = api::routes()
        .layer(TraceLayer::new_for_http())
        .layer(cors)
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(&args.addr).await?;
    tracing::info!(addr = %args.addr, "cairn-server listening");
    axum::serve(listener, app).await?;
    Ok(())
}

/// Run one operator command and return.
async fn run_command(pool: &PgPool, command: &Command) -> anyhow::Result<()> {
    match command {
        Command::Users(UserCommand::Add {
            email,
            display_name,
            password,
            must_change_password,
        }) => {
            // The same `auth::create_user` the HTTP route calls. One mechanism,
            // two entry points: an operator with shell access, and an
            // administrator with a token. Neither is a second implementation of
            // account creation, which is what keeps validation and hashing from
            // drifting apart between them.
            let (id, email) =
                auth::create_user(pool, email, display_name, password, *must_change_password)
                    .await?;
            // One line of JSON on stdout: this is a seam for scripts and test
            // harnesses as much as for a human, and a human reading one line of
            // JSON is a smaller cost than a script parsing prose.
            println!("{}", serde_json::json!({ "id": id, "email": email }));
            Ok(())
        }
    }
}

/// Apply the environment-defined account, before the listener binds.
///
/// A deployment whose operator credentials are malformed has nobody able to
/// sign in, so it fails to start rather than serving traffic that turns anyone
/// away. Half a pair is a mistake worth the same treatment: it reads as
/// configured but seeds nothing.
async fn seed_admin(pool: &PgPool, args: &Args) -> anyhow::Result<()> {
    match (
        configured(args.admin_email.as_deref()),
        configured(args.admin_password.as_deref()),
    ) {
        (Some(email), Some(password)) => {
            let (id, outcome) =
                auth::ensure_admin(pool, email, args.admin_display_name.trim(), password).await?;
            tracing::info!(
                user = %id,
                email = %email.trim().to_lowercase(),
                outcome = outcome.as_str(),
                "admin account defined by the environment"
            );
            Ok(())
        }
        (None, None) => Ok(()),
        (Some(_), None) => {
            anyhow::bail!("CAIRN_ADMIN_EMAIL is set but CAIRN_ADMIN_PASSWORD is not")
        }
        (None, Some(_)) => {
            anyhow::bail!("CAIRN_ADMIN_PASSWORD is set but CAIRN_ADMIN_EMAIL is not")
        }
    }
}

/// An empty variable is how a `.env` file and a Compose `environment:` block
/// spell "unset" — the process still receives the name, just with nothing in
/// it. Treating that as configured would fail every default deployment.
fn configured(value: Option<&str>) -> Option<&str> {
    value.filter(|v| !v.trim().is_empty())
}

fn init_tracing() {
    use tracing_subscriber::{fmt, EnvFilter};
    let filter = EnvFilter::try_from_env("CAIRN_LOG")
        .unwrap_or_else(|_| EnvFilter::new("info,tower_http=warn"));
    let _ = fmt().with_env_filter(filter).try_init();
}
