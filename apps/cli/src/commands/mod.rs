pub mod daemon;
pub mod init;
pub mod project;
pub mod session;
pub mod status;
pub mod task;

pub fn cwd() -> String {
    std::env::current_dir()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| ".".to_string())
}
