use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct Config {
    pub data_dir: PathBuf,
    pub db_path: PathBuf,
    pub listen_addr: String,
    pub listen_port: u16,
    pub secure_cookies: bool,
    pub version_history_count: usize,
    pub max_body_size: usize,
    pub allow_private_webhooks: bool,
    pub registrations_enabled: bool,
    /// When true, trust X-Forwarded-For headers for rate limiting.
    /// Only enable this when running behind a trusted reverse proxy.
    /// When false (default), rate limiting uses a single global bucket.
    pub trust_proxy_headers: bool,
}

impl Config {
    pub fn new(
        data_dir: Option<PathBuf>,
        db_path: Option<PathBuf>,
        port: Option<u16>,
        secure_cookies: bool,
        version_history_count: usize,
        max_body_size_mb: usize,
    ) -> Self {
        let data_dir = data_dir.unwrap_or_else(|| PathBuf::from("data"));
        let db_path = db_path.unwrap_or_else(|| data_dir.join("substrukt.db"));
        Self {
            data_dir,
            db_path,
            listen_addr: "0.0.0.0".into(),
            listen_port: port.unwrap_or(3000),
            secure_cookies,
            version_history_count,
            max_body_size: max_body_size_mb * 1024 * 1024,
            allow_private_webhooks: false,
            registrations_enabled: false,
            trust_proxy_headers: false,
        }
    }

    pub fn schemas_dir(&self) -> PathBuf {
        self.data_dir.join("schemas")
    }

    pub fn content_dir(&self) -> PathBuf {
        self.data_dir.join("content")
    }

    pub fn uploads_dir(&self) -> PathBuf {
        self.data_dir.join("uploads")
    }

    pub fn ensure_dirs(&self) -> eyre::Result<()> {
        std::fs::create_dir_all(self.schemas_dir())?;
        std::fs::create_dir_all(self.content_dir())?;
        std::fs::create_dir_all(self.uploads_dir())?;
        Ok(())
    }

    // --- App-scoped path helpers ---

    pub fn app_dir(&self, app_slug: &str) -> PathBuf {
        self.data_dir.join(app_slug)
    }

    pub fn app_schemas_dir(&self, app_slug: &str) -> PathBuf {
        self.app_dir(app_slug).join("schemas")
    }

    pub fn app_content_dir(&self, app_slug: &str) -> PathBuf {
        self.app_dir(app_slug).join("content")
    }

    pub fn app_uploads_dir(&self, app_slug: &str) -> PathBuf {
        self.app_dir(app_slug).join("uploads")
    }

    pub fn app_history_dir(&self, app_slug: &str) -> PathBuf {
        self.app_dir(app_slug).join("_history")
    }

    /// Create the directory structure for a new app.
    pub fn ensure_app_dirs(&self, app_slug: &str) -> eyre::Result<()> {
        std::fs::create_dir_all(self.app_schemas_dir(app_slug))?;
        std::fs::create_dir_all(self.app_content_dir(app_slug))?;
        std::fs::create_dir_all(self.app_uploads_dir(app_slug))?;
        std::fs::create_dir_all(self.app_history_dir(app_slug))?;
        Ok(())
    }
}
