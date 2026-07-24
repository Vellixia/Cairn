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
pub const SESSION_BIND: &str = "v1.session.bind";
pub const EVENTS_LIST: &str = "v1.events.list";
pub const PROJECT_CREATE: &str = "v1.project.create";
pub const PROJECT_LIST: &str = "v1.project.list";
pub const PROJECT_GET: &str = "v1.project.get";
pub const PROJECT_UPDATE: &str = "v1.project.update";
pub const PROJECT_REPOSITORY_ASSOCIATE: &str = "v1.project.repository_associate";
pub const TASK_CREATE: &str = "v1.task.create";
pub const TASK_REVISE: &str = "v1.task.revise";
pub const TASK_LIST: &str = "v1.task.list";
pub const TASK_GET: &str = "v1.task.get";

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
    SESSION_BIND,
    EVENTS_LIST,
    PROJECT_CREATE,
    PROJECT_LIST,
    PROJECT_GET,
    PROJECT_UPDATE,
    PROJECT_REPOSITORY_ASSOCIATE,
    TASK_CREATE,
    TASK_REVISE,
    TASK_LIST,
    TASK_GET,
];

/// Cross-surface inventory for every Feature 002 method and Feature 001 method
/// extended by Feature 002. Contract tests use this as the authoritative list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Feature002Method {
    pub method: &'static str,
    pub params_schema: &'static str,
    pub result_schema: &'static str,
    pub cli_command: Option<&'static str>,
}

pub const FEATURE002_METHODS: &[Feature002Method] = &[
    Feature002Method {
        method: PROJECT_CREATE,
        params_schema: "project-create-params",
        result_schema: "project-create-result",
        cli_command: Some("project.create"),
    },
    Feature002Method {
        method: PROJECT_LIST,
        params_schema: "project-list-params",
        result_schema: "project-list-result",
        cli_command: Some("project.list"),
    },
    Feature002Method {
        method: PROJECT_GET,
        params_schema: "project-get-params",
        result_schema: "project-get-result",
        cli_command: Some("project.show"),
    },
    Feature002Method {
        method: PROJECT_UPDATE,
        params_schema: "project-update-params",
        result_schema: "project-update-result",
        cli_command: Some("project.update"),
    },
    Feature002Method {
        method: PROJECT_REPOSITORY_ASSOCIATE,
        params_schema: "project-repository-associate-params",
        result_schema: "project-repository-associate-result",
        cli_command: Some("project.repository.add"),
    },
    Feature002Method {
        method: TASK_CREATE,
        params_schema: "task-create-params",
        result_schema: "task-create-result",
        cli_command: Some("task.create"),
    },
    Feature002Method {
        method: TASK_REVISE,
        params_schema: "task-revise-params",
        result_schema: "task-revise-result",
        cli_command: Some("task.revise"),
    },
    Feature002Method {
        method: TASK_LIST,
        params_schema: "task-list-params",
        result_schema: "task-list-result",
        cli_command: Some("task.list"),
    },
    Feature002Method {
        method: TASK_GET,
        params_schema: "task-get-params",
        result_schema: "task-get-result",
        cli_command: Some("task.show"),
    },
    Feature002Method {
        method: SESSION_BIND,
        params_schema: "session-bind-params",
        result_schema: "session-bind-result",
        cli_command: Some("session.bind"),
    },
    Feature002Method {
        method: SESSION_START,
        params_schema: "session-start-params",
        result_schema: "session-start-result",
        cli_command: Some("session.start"),
    },
    Feature002Method {
        method: SESSION_GET,
        params_schema: "session-get-params",
        result_schema: "session-get-result",
        cli_command: Some("session.show"),
    },
    Feature002Method {
        method: SESSION_LIST,
        params_schema: "session-list-params",
        result_schema: "session-list-result",
        cli_command: Some("session.list"),
    },
    Feature002Method {
        method: EVENTS_LIST,
        params_schema: "events-list-params",
        result_schema: "events-list-result",
        cli_command: None,
    },
];
