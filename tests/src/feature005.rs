//! Shared fixtures for Feature 005 — Server-Authoritative Autonomous Memory.
//!
//! Four things this feature needs that the existing harness does not provide,
//! collected here so no story rebuilds them:
//!
//! 1. **A PostgreSQL fixture at server schema v4** with a project and accounts
//!    already seeded, because almost every Feature 005 assertion is about what
//!    an *authenticated* request may do to a project it may or may not belong
//!    to.
//! 2. **Identical-UUID seed helpers.** `reference_key` must carry the domain,
//!    and the only way to prove it does is to give `project`, `personal`,
//!    `team` and `pattern` records *deliberately the same* UUID and show
//!    nothing collapses them (SC-766, SC-767). A helper that mints random ids
//!    cannot test that.
//! 3. **Authenticated multi-account helpers.** Owner, co-member and outsider,
//!    with tokens, because privacy and ownership are the feature's sharpest
//!    edges and a single-account fixture cannot see them.
//! 4. **Restart injection.** Consolidation leases, spool claims and the
//!    fifth-attempt rule are all statements about what survives a process
//!    dying mid-flight, so the harness has to be able to kill one.
//!
//! Every constructor that needs PostgreSQL returns `Option`, matching the
//! existing `Server` convention: a checkout without `CAIRN_TEST_DATABASE_URL`
//! reports a skip rather than passing vacuously.

use crate::Server;
use std::path::PathBuf;
use tempfile::TempDir;
use uuid::Uuid;

/// The local schema version Feature 005 introduces (`data-model.md` §5).
pub const LOCAL_SCHEMA_V8: i64 = 8;
/// The local schema version Feature 005 upgrades *from*.
pub const LOCAL_SCHEMA_V7: i64 = 7;
/// The server schema version Feature 005 introduces (`data-model.md` §6).
pub const SERVER_SCHEMA_V4: i64 = 4;
/// The server schema version Feature 005 upgrades *from*.
pub const SERVER_SCHEMA_V3: i64 = 3;

/// The one password every seeded account uses.
///
/// Shared deliberately: these fixtures test authorization, not credentials, and
/// a per-account password would only add a lookup to every helper.
pub const PASSWORD: &str = "hunter2hunter2";

// ---------------------------------------------------------------------------
// Accounts
// ---------------------------------------------------------------------------

/// A seeded account and the bearer token that authenticates as it.
#[derive(Debug, Clone)]
pub struct Account {
    pub id: Uuid,
    pub email: String,
    pub token: String,
}

// ---------------------------------------------------------------------------
// The PostgreSQL fixture
// ---------------------------------------------------------------------------

/// A server on a database of its own, at schema v4, with three accounts and a
/// project.
///
/// A database of its own rather than the shared one, because Feature 005 tests
/// count rows in tables that are global to a server — consolidation work,
/// leases, health — and a shared database would let one test's backlog decide
/// another's assertion.
pub struct Pg {
    pub server: Server,
    /// The project all three accounts are measured against.
    pub project: Uuid,
    /// A member. Owns the personal knowledge and the patterns in the fixture.
    pub owner: Account,
    /// A second member of the same project. Sees project and team knowledge,
    /// must never see the owner's personal knowledge or patterns.
    pub member: Account,
    /// Authenticated, but a member of nothing. Every project-scoped route must
    /// refuse this account.
    pub outsider: Account,
}

impl Pg {
    /// The standard fixture: v4, one project, owner + member + outsider.
    pub fn start() -> Option<Self> {
        let server = Server::start_own_database()?;
        Some(Self::seed(server))
    }

    /// A server deliberately pinned at v3, for the migration story (US7).
    ///
    /// Nothing is seeded: a v3 database has no Feature 005 tables to seed into,
    /// and the point of the fixture is to fill it the way a v3 installation
    /// actually would and then migrate it.
    pub fn start_at_v3() -> Option<Server> {
        Server::start_at_schema(SERVER_SCHEMA_V3)
    }

    fn seed(server: Server) -> Self {
        let owner = account(&server, "owner");
        let member = account(&server, "member");
        let outsider = account(&server, "outsider");

        let project = Uuid::now_v7();
        server.execute(&format!(
            "INSERT INTO projects (id, name, repository_remote)
             VALUES ('{project}', 'feature005-fixture', 'git@example.test:feature005.git')"
        ));
        for who in [&owner, &member] {
            server.execute(&format!(
                "INSERT INTO project_members (project_id, user_id)
                 VALUES ('{project}', '{}')",
                who.id
            ));
        }

        Self {
            server,
            project,
            owner,
            member,
            outsider,
        }
    }

    /// A fourth account, for a test that needs more than the standard three.
    ///
    /// Not a member of the fixture project unless `member` is true.
    pub fn extra_account(&self, label: &str, member: bool) -> Account {
        let who = account(&self.server, label);
        if member {
            self.server.execute(&format!(
                "INSERT INTO project_members (project_id, user_id)
                 VALUES ('{}', '{}')",
                self.project, who.id
            ));
        }
        who
    }

    /// A second project, so a test can prove a route does not leak across one.
    pub fn extra_project(&self, name: &str, members: &[&Account]) -> Uuid {
        let id = Uuid::now_v7();
        self.server.execute(&format!(
            "INSERT INTO projects (id, name) VALUES ('{id}', '{name}')"
        ));
        for who in members {
            self.server.execute(&format!(
                "INSERT INTO project_members (project_id, user_id) VALUES ('{id}', '{}')",
                who.id
            ));
        }
        id
    }

    /// A session row owned by `who` in the fixture project.
    ///
    /// Safe-event ingest verifies server-side that the session an event names
    /// really belongs to the authenticated account (FR-768, Principle XI), so
    /// nearly every ingest test needs one of these and one belonging to
    /// somebody else.
    pub fn session_for(&self, who: &Account) -> Uuid {
        self.session_in(self.project, who)
    }

    pub fn session_in(&self, project: Uuid, who: &Account) -> Uuid {
        let id = Uuid::now_v7();
        // `sessions.user_id` is the account binding, and it is what a
        // server-side ownership check reads (FR-768). It is nullable in the
        // schema — a pre-Feature-005 row may not have one — so the fixture
        // always sets it, because a session whose owner is unknown cannot be
        // the subject of an authorization assertion either way.
        self.server.execute(&format!(
            "INSERT INTO sessions (id, project_id, user_id, agent, branch, status, started_at)
             VALUES ('{id}', '{project}', '{}', 'claude-code', 'main', 'active', now())",
            who.id
        ));
        id
    }

    /// The server's schema version, read from the database rather than assumed.
    pub fn schema_version(&self) -> i64 {
        self.server
            .count("SELECT COALESCE(MAX(version), 0) FROM schema_migrations")
    }

    /// Kill this server and stand a replacement up at the same address on the
    /// same data — the restart injection control.
    ///
    /// `Server::upgraded_in_place` already does exactly this (SIGKILL, then
    /// respawn on the same database and port), which is what a crash mid-batch
    /// looks like to everything outside the process. Wrapped rather than
    /// reimplemented so there is one restart path in the harness, and named for
    /// what Feature 005 uses it for: nothing here is an upgrade.
    pub fn crash_and_restart(&mut self) {
        let replacement = self.server.upgraded_in_place();
        self.server = replacement;
    }
}

fn account(server: &Server, label: &str) -> Account {
    // `new_user` already mints a bearer token, which is the credential every
    // Feature 005 route authenticates with. Reading the email back rather than
    // reconstructing it keeps the fixture honest about the account that exists.
    let (id, token) = server.new_user(label);
    let email = server
        .query_column(&format!("SELECT email FROM users WHERE id = '{id}'"))
        .first()
        .cloned()
        .unwrap_or_else(|| panic!("the account just created has no row: {label}"));
    Account { id, email, token }
}

// ---------------------------------------------------------------------------
// Identical-UUID seeding (SC-766, SC-767)
// ---------------------------------------------------------------------------

/// One UUID, deliberately reused as the id of a record in every domain.
///
/// This is the adversarial input for `reference_key`: if any part of the system
/// keys a reference on the bare UUID, these four records become one, and a
/// personal record is served where a project record was asked for. Every
/// Feature 005 surface that carries a reference is expected to keep them apart.
#[derive(Debug, Clone)]
pub struct IdenticalIds {
    /// The single UUID all four records share.
    pub id: Uuid,
    /// `project:<id>` — a row in `memories`.
    pub project_memory: Uuid,
    /// `personal:<id>` — a row in `personal_knowledge`, owned by `owner`.
    pub personal: Uuid,
    /// `team:<id>` — a row in `team_knowledge`.
    pub team: Uuid,
    /// `pattern:<id>` — a row in `shared_patterns`, owned by `owner`.
    pub pattern: Uuid,
}

impl IdenticalIds {
    /// The four canonical reference keys these records must produce
    /// (`data-model.md` §6.1).
    ///
    /// Three `knowledge:<domain>:<id>` keys and one `pattern:<id>`. The
    /// pattern's key omits a domain component because a `PatternRef` is not a
    /// `KnowledgeRef`, not because the pattern has no domain — the
    /// `shared_patterns` row it names carries `domain = 'personal'`.
    pub fn reference_keys(&self) -> [String; 4] {
        [
            format!("knowledge:project:{}", self.id),
            format!("knowledge:personal:{}", self.id),
            format!("knowledge:team:{}", self.id),
            format!("pattern:{}", self.id),
        ]
    }
}

impl Pg {
    /// Seed one record in each domain, all four sharing a single UUID.
    ///
    /// `shared_patterns` does not exist before T007, so the pattern row is
    /// inserted conditionally: a test that runs against a v3 database still
    /// gets the other three rather than failing on a missing table.
    pub fn seed_identical_ids(&self, owner: &Account) -> IdenticalIds {
        let id = Uuid::now_v7();
        let session = self.session_for(owner);

        self.server.execute(&format!(
            "INSERT INTO memories
                 (id, project_id, type, scope, scope_key, content, origin_session_id)
             VALUES ('{id}', '{}', 'fact', 'project', '{}',
                     'project-domain record with the shared id', '{session}')",
            self.project, self.project
        ));

        self.server.execute(&format!(
            "INSERT INTO personal_knowledge
                 (id, owner_user_id, knowledge_type, content, writer_id, writer_seq)
             VALUES ('{id}', '{}', 'fact',
                     'personal-domain record with the shared id',
                     'feature005-fixture-{id}', 1)",
            owner.id
        ));

        self.server.execute(&format!(
            "INSERT INTO team_knowledge
                 (id, knowledge_type, content, proposed_by_user_id, writer_id, writer_seq)
             VALUES ('{id}', 'fact', 'team-domain record with the shared id',
                     '{}', 'feature005-fixture-{id}', 2)",
            owner.id
        ));

        self.seed_pattern_with_id(owner, id, "pattern record with the shared id");

        IdenticalIds {
            id,
            project_memory: id,
            personal: id,
            team: id,
            pattern: id,
        }
    }

    /// Insert a `shared_patterns` row, if the table exists yet.
    ///
    /// Returns whether it did, so a caller that genuinely requires the row can
    /// assert rather than silently proceed without it.
    pub fn seed_pattern_with_id(&self, owner: &Account, id: Uuid, content: &str) -> bool {
        if !self.table_exists("shared_patterns") {
            return false;
        }
        // The primary key is `pattern_id`, and the content is four columns
        // rather than one: a pattern is a problem, a cause and an approach,
        // with the title as a label. `content_key` is what makes a repeat
        // promotion an upsert, so it is derived from the id here to keep two
        // fixture patterns distinct.
        let safe = content.replace('\'', "''");
        self.server.execute(&format!(
            "INSERT INTO shared_patterns
                 (pattern_id, owner_user_id, domain, title, problem, root_cause,
                  approach, content_key)
             VALUES ('{id}', '{}', 'personal', '{safe}', '{safe} problem',
                     '{safe} root cause', '{safe} approach', 'content-key-{id}')",
            owner.id
        ));
        true
    }

    /// Run a statement, returning the database's error instead of panicking.
    ///
    /// A schema test's sharpest assertions are about what the database
    /// *refuses*, and `Server::execute` panics on refusal — which is correct
    /// for seeding a fixture and useless for testing a constraint.
    pub fn try_execute(&self, sql: &str) -> Result<(), String> {
        let url = self.server.database_url.clone();
        let sql = sql.to_string();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        runtime.block_on(async move {
            let pool = sqlx::PgPool::connect(&url).await.expect("open server db");
            let outcome = sqlx::query(&sql).execute(&pool).await.map(|_| ());
            pool.close().await;
            outcome.map_err(|e| e.to_string())
        })
    }

    /// Assert a statement is refused, and say what was being tested when it is
    /// not.
    pub fn refuses(&self, what: &str, sql: &str) {
        if self.try_execute(sql).is_ok() {
            panic!("the schema accepted {what}");
        }
    }

    /// Whether an index of this name exists.
    pub fn index_exists(&self, name: &str) -> bool {
        self.server.count(&format!(
            "SELECT count(*) FROM pg_indexes
              WHERE schemaname = 'public' AND indexname = '{name}'"
        )) > 0
    }

    pub fn table_exists(&self, name: &str) -> bool {
        self.server.count(&format!(
            "SELECT count(*) FROM information_schema.tables
              WHERE table_schema = 'public' AND table_name = '{name}'"
        )) > 0
    }

    pub fn column_exists(&self, table: &str, column: &str) -> bool {
        self.server.count(&format!(
            "SELECT count(*) FROM information_schema.columns
              WHERE table_schema = 'public'
                AND table_name = '{table}' AND column_name = '{column}'"
        )) > 0
    }
}

// ---------------------------------------------------------------------------
// The SQLite fixture
// ---------------------------------------------------------------------------

/// A local store on disk at the current schema, with a project seeded.
///
/// On disk rather than in memory because Feature 005's local story is about
/// what survives a restart and what a deleted store loses, and neither is
/// observable against a database that dies with its connection. For a store
/// pinned *below* the current schema, see [`LocalAt`].
pub struct Local {
    dir: TempDir,
    pub store: cairn_store::Store,
    pub project: Uuid,
}

impl Local {
    /// A store migrated to the newest version this build knows.
    pub async fn new() -> Self {
        let dir = TempDir::new().expect("a directory for the local store");
        let store = cairn_store::Store::open(&dir.path().join("cairn.sqlite3"))
            .await
            .expect("open store");
        let project = seed_local_project(store.pool()).await;
        Self {
            dir,
            store,
            project,
        }
    }

    /// The same synchronous entry point `store_fixture` offers, for tests that
    /// are not themselves async.
    pub fn blocking() -> (tokio::runtime::Runtime, Self) {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        let f = rt.block_on(Self::new());
        (rt, f)
    }

    pub fn path(&self) -> PathBuf {
        self.dir.path().join("cairn.sqlite3")
    }

    pub async fn schema_version(&self) -> i64 {
        sqlx::query_scalar::<_, i64>("SELECT COALESCE(MAX(version), 0) FROM schema_migrations")
            .fetch_one(self.store.pool())
            .await
            .expect("schema version")
    }

    pub async fn count(&self, sql: &str) -> i64 {
        sqlx::query_scalar::<_, i64>(sql)
            .fetch_one(self.store.pool())
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"))
    }

    pub async fn execute(&self, sql: &str) {
        sqlx::query(sql)
            .execute(self.store.pool())
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"));
    }

    /// Whether a table exists — the local counterpart of `Pg::table_exists`.
    pub async fn table_exists(&self, name: &str) -> bool {
        self.count(&format!(
            "SELECT count(*) FROM sqlite_master WHERE type = 'table' AND name = '{name}'"
        ))
        .await
            > 0
    }

    /// Whether a column exists on a table.
    pub async fn column_exists(&self, table: &str, column: &str) -> bool {
        let cols: Vec<String> =
            sqlx::query_scalar(&format!("SELECT name FROM pragma_table_info('{table}')"))
                .fetch_all(self.store.pool())
                .await
                .unwrap_or_default();
        cols.iter().any(|c| c == column)
    }

    /// Close this store and reopen the same file, letting the build's
    /// migrations run — the local restart injection control, and the shape of
    /// a v7→v8 upgrade.
    pub async fn reopen_migrated(self) -> Self {
        let Self {
            dir,
            store,
            project,
        } = self;
        store.pool().close().await;
        let path = dir.path().join("cairn.sqlite3");
        let store = cairn_store::Store::open(&path)
            .await
            .expect("reopen and migrate the local store");
        Self {
            dir,
            store,
            project,
        }
    }
}

/// Seed the one project every local fixture measures against.
async fn seed_local_project(pool: &sqlx::SqlitePool) -> Uuid {
    let project = Uuid::now_v7();
    let now = "2026-09-02T00:00:00Z";
    sqlx::query(
        "INSERT INTO projects (id, name, git_common_dir, repository_remote, linked,
                               server_project_id, created_at, updated_at, deleted_at)
         VALUES (?1, 'feature005-fixture', ?2, NULL, 0, NULL, ?3, ?3, NULL)",
    )
    .bind(project.to_string())
    .bind(format!("/fixture/{project}/.git"))
    .bind(now)
    .execute(pool)
    .await
    .expect("seed the fixture project");
    project
}

// ---------------------------------------------------------------------------
// A local store pinned below the current schema
// ---------------------------------------------------------------------------

/// A local database stopped at a chosen schema version.
///
/// Deliberately not a [`Store`](cairn_store::Store): `Store::open` migrates to
/// the newest version this build knows, so a `Store` at v7 is not a thing that
/// can exist. What a v7 installation actually is, is a *file* — so that is what
/// this holds, built by running the real migrations up to v7 rather than by
/// hand-writing an approximation of the schema someone remembers v7 having.
///
/// [`migrate_to_latest`](Self::migrate_to_latest) is then the upgrade under
/// test: the same file, opened by the current build.
pub struct LocalAt {
    dir: TempDir,
    pub pool: sqlx::SqlitePool,
    pub project: Uuid,
}

impl LocalAt {
    /// A local database migrated up to, and stopping at, `version`.
    pub async fn new(version: i64) -> Self {
        let dir = TempDir::new().expect("a directory for the local store");
        let path = dir.path().join("cairn.sqlite3");
        let pool = open_pool(&path).await;
        cairn_store::migrate::run_to(&pool, version)
            .await
            .unwrap_or_else(|e| panic!("migrating a fixture store to v{version}: {e}"));
        let project = seed_local_project(&pool).await;
        Self { dir, pool, project }
    }

    pub fn path(&self) -> PathBuf {
        self.dir.path().join("cairn.sqlite3")
    }

    pub async fn schema_version(&self) -> i64 {
        sqlx::query_scalar::<_, i64>("SELECT COALESCE(MAX(version), 0) FROM schema_migrations")
            .fetch_one(&self.pool)
            .await
            .expect("schema version")
    }

    pub async fn count(&self, sql: &str) -> i64 {
        sqlx::query_scalar::<_, i64>(sql)
            .fetch_one(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"))
    }

    pub async fn execute(&self, sql: &str) {
        sqlx::query(sql)
            .execute(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"));
    }

    /// Attempt a statement, returning the error rather than panicking.
    ///
    /// A schema test's most important assertions are the ones about what the
    /// database *refuses*, and a helper that panics on failure cannot make
    /// them.
    pub async fn try_execute(&self, sql: &str) -> Result<(), sqlx::Error> {
        sqlx::query(sql).execute(&self.pool).await.map(|_| ())
    }

    /// Run the remaining migrations against this same file.
    ///
    /// The pool is closed first: this is one process replacing another over one
    /// database file, and leaving the old connections open would test something
    /// no upgrade ever does.
    pub async fn migrate_to_latest(self) -> Local {
        let Self { dir, pool, project } = self;
        pool.close().await;
        let store = cairn_store::Store::open(&dir.path().join("cairn.sqlite3"))
            .await
            .expect("the current build opens and migrates a pinned store");
        Local {
            dir,
            store,
            project,
        }
    }

    /// Migrate to `version`, returning the error instead of panicking.
    ///
    /// The interrupted-migration case needs this: what matters is that a
    /// migration which fails part way leaves the database on its old version
    /// rather than half way between two.
    pub async fn try_migrate_to(&self, version: i64) -> Result<i64, String> {
        cairn_store::migrate::run_to(&self.pool, version)
            .await
            .map_err(|e| e.to_string())
    }
}

/// The same connection options `Store::open` uses, for a pool this harness owns.
///
/// Copied rather than shared because `Store::open_with_busy_timeout` is crate
/// private and widening it for a fixture would put a test-only seam in the
/// production API.
async fn open_pool(path: &std::path::Path) -> sqlx::SqlitePool {
    use sqlx::sqlite::{
        SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous,
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("store directory");
    }
    let options = SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal)
        .foreign_keys(true)
        .busy_timeout(std::time::Duration::from_secs(5))
        .pragma("secure_delete", "ON");
    SqlitePoolOptions::new()
        .max_connections(8)
        .acquire_timeout(std::time::Duration::from_secs(10))
        .connect_with(options)
        .await
        .expect("open a pinned-version store")
}
