//! v1 IPC method names.

pub const DAEMON_STATUS: &str = "v1.daemon.status";
pub const REPOSITORY_REGISTER: &str = "v1.repository.register";
pub const REPOSITORY_INSPECT: &str = "v1.repository.inspect";
pub const REPOSITORY_IGNORED_FILES: &str = "v1.repository.ignored_files";
pub const SNAPSHOT_CREATE: &str = "v1.snapshot.create";
pub const SESSION_START: &str = "v1.session.start";
pub const SESSION_GET: &str = "v1.session.get";
pub const SESSION_LIST: &str = "v1.session.list";
pub const SESSION_HEARTBEAT: &str = "v1.session.heartbeat";
pub const SESSION_REATTACH: &str = "v1.session.reattach";
pub const SESSION_STOP: &str = "v1.session.stop";
pub const EVENTS_LIST: &str = "v1.events.list";

/// All v1 methods (used by router registration checks and contract tests).
pub const ALL_METHODS: &[&str] = &[
    DAEMON_STATUS,
    REPOSITORY_REGISTER,
    REPOSITORY_INSPECT,
    REPOSITORY_IGNORED_FILES,
    SNAPSHOT_CREATE,
    SESSION_START,
    SESSION_GET,
    SESSION_LIST,
    SESSION_HEARTBEAT,
    SESSION_REATTACH,
    SESSION_STOP,
    EVENTS_LIST,
];
