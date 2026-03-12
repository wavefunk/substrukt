use std::sync::Arc;

use dashmap::DashMap;
use reqwest::{Client, StatusCode, redirect};
use tokio::net::TcpListener;
use tokio::sync::RwLock;
use tower_sessions::SessionManagerLayer;
use tower_sessions_sqlx_store::SqliteStore;

use substrukt::cache;
use substrukt::config::Config;
use substrukt::db;
use substrukt::routes;
use substrukt::state::AppStateInner;
use substrukt::templates;

struct TestServer {
    base_url: String,
    client: Client,
    _data_dir: tempfile::TempDir,
    _shutdown: tokio::sync::oneshot::Sender<()>,
}

impl TestServer {
    async fn start() -> Self {
        let data_dir = tempfile::tempdir().unwrap();
        let db_path = data_dir.path().join("test.db");
        let config = Config::new(
            Some(data_dir.path().to_path_buf()),
            Some(db_path),
            Some(0), // port 0 = OS assigns random port
        );
        config.ensure_dirs().unwrap();

        let pool = db::init_pool(&config.db_path).await.unwrap();
        let session_store = SqliteStore::new(pool.clone());
        session_store.migrate().await.unwrap();
        let session_layer = SessionManagerLayer::new(session_store).with_secure(false);

        let env = templates::create_environment(config.schemas_dir());
        let content_cache = DashMap::new();
        cache::populate(&content_cache, &config.schemas_dir(), &config.content_dir());

        let state = Arc::new(AppStateInner {
            pool,
            config,
            templates: RwLock::new(env),
            cache: content_cache,
        });

        let app = routes::build_router(state).layer(session_layer);
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let base_url = format!("http://{addr}");

        let (tx, rx) = tokio::sync::oneshot::channel::<()>();
        tokio::spawn(async move {
            axum::serve(listener, app)
                .with_graceful_shutdown(async { rx.await.ok(); })
                .await
                .unwrap();
        });

        let client = Client::builder()
            .cookie_store(true)
            .redirect(redirect::Policy::none())
            .build()
            .unwrap();

        TestServer {
            base_url,
            client,
            _data_dir: data_dir,
            _shutdown: tx,
        }
    }

    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base_url)
    }

    async fn setup_admin(&self) {
        self.client
            .post(self.url("/setup"))
            .form(&[
                ("username", "admin"),
                ("password", "testpassword"),
                ("confirm_password", "testpassword"),
            ])
            .send()
            .await
            .unwrap();
    }

    async fn create_schema(&self, json: &str) {
        self.client
            .post(self.url("/schemas/new"))
            .form(&[("schema_json", json)])
            .send()
            .await
            .unwrap();
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        // shutdown is consumed on drop, but we can't send twice
        // This is handled by the oneshot channel being dropped
    }
}

// ── Auth tests ───────────────────────────────────────────────

#[tokio::test]
async fn auth_redirects_to_setup_when_no_users() {
    let s = TestServer::start().await;
    let resp = s.client.get(s.url("/")).send().await.unwrap();
    assert_eq!(resp.status(), StatusCode::SEE_OTHER);
    assert_eq!(resp.headers().get("location").unwrap(), "/setup");
}

#[tokio::test]
async fn auth_setup_creates_admin_and_sets_session() {
    let s = TestServer::start().await;
    let resp = s.client.post(s.url("/setup"))
        .form(&[
            ("username", "admin"),
            ("password", "testpassword"),
            ("confirm_password", "testpassword"),
        ])
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::SEE_OTHER);
    assert_eq!(resp.headers().get("location").unwrap(), "/");

    // Session should now work
    let resp = s.client.get(s.url("/")).send().await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn auth_setup_rejects_when_user_exists() {
    let s = TestServer::start().await;
    s.setup_admin().await;

    let resp = s.client.get(s.url("/setup")).send().await.unwrap();
    assert_eq!(resp.status(), StatusCode::SEE_OTHER);
    assert_eq!(resp.headers().get("location").unwrap(), "/login");
}

#[tokio::test]
async fn auth_login_and_logout() {
    let s = TestServer::start().await;
    s.setup_admin().await;

    // Logout
    let resp = s.client.post(s.url("/logout")).send().await.unwrap();
    assert_eq!(resp.status(), StatusCode::SEE_OTHER);

    // Should redirect to login now
    let resp = s.client.get(s.url("/")).send().await.unwrap();
    assert_eq!(resp.status(), StatusCode::SEE_OTHER);
    assert_eq!(resp.headers().get("location").unwrap(), "/login");

    // Login again
    let resp = s.client.post(s.url("/login"))
        .form(&[("username", "admin"), ("password", "testpassword")])
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::SEE_OTHER);
    assert_eq!(resp.headers().get("location").unwrap(), "/");
}

// ── Schema CRUD tests ────────────────────────────────────────

const BLOG_SCHEMA: &str = r#"{
    "x-substrukt": {"title": "Blog Posts", "slug": "blog-posts", "storage": "directory"},
    "type": "object",
    "properties": {
        "title": {"type": "string", "title": "Title"},
        "body": {"type": "string", "title": "Body", "format": "textarea"},
        "published": {"type": "boolean", "title": "Published"}
    },
    "required": ["title"]
}"#;

#[tokio::test]
async fn schema_create_and_list() {
    let s = TestServer::start().await;
    s.setup_admin().await;

    let resp = s.client.post(s.url("/schemas/new"))
        .form(&[("schema_json", BLOG_SCHEMA)])
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::SEE_OTHER);

    let resp = s.client.get(s.url("/schemas")).send().await.unwrap();
    let body = resp.text().await.unwrap();
    assert!(body.contains("Blog Posts"));
}

#[tokio::test]
async fn schema_edit_and_update() {
    let s = TestServer::start().await;
    s.setup_admin().await;
    s.create_schema(BLOG_SCHEMA).await;

    // Edit page loads
    let resp = s.client.get(s.url("/schemas/blog-posts/edit")).send().await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // Update via POST
    let updated = BLOG_SCHEMA.replace("Blog Posts", "Articles");
    let resp = s.client.post(s.url("/schemas/blog-posts"))
        .form(&[("schema_json", updated.as_str())])
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::SEE_OTHER);
}

#[tokio::test]
async fn schema_delete() {
    let s = TestServer::start().await;
    s.setup_admin().await;
    s.create_schema(BLOG_SCHEMA).await;

    let resp = s.client.delete(s.url("/schemas/blog-posts")).send().await.unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);
}

// ── Content CRUD tests ───────────────────────────────────────

#[tokio::test]
async fn content_create_and_list() {
    let s = TestServer::start().await;
    s.setup_admin().await;
    s.create_schema(BLOG_SCHEMA).await;

    // New entry page
    let resp = s.client.get(s.url("/content/blog-posts/new")).send().await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = resp.text().await.unwrap();
    assert!(body.contains("<input"), "Form should have input fields");
    assert!(body.contains("<textarea"), "Form should have textarea");

    // Create entry
    let form = reqwest::multipart::Form::new()
        .text("title", "Hello World")
        .text("body", "First post")
        .text("published", "true");
    let resp = s.client.post(s.url("/content/blog-posts/new"))
        .multipart(form)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::SEE_OTHER);

    // Entry appears in list
    let resp = s.client.get(s.url("/content/blog-posts")).send().await.unwrap();
    let body = resp.text().await.unwrap();
    assert!(body.contains("Hello World"));
}

#[tokio::test]
async fn content_edit_and_delete() {
    let s = TestServer::start().await;
    s.setup_admin().await;
    s.create_schema(BLOG_SCHEMA).await;

    // Create
    let form = reqwest::multipart::Form::new()
        .text("title", "To Edit")
        .text("body", "Original");
    s.client.post(s.url("/content/blog-posts/new"))
        .multipart(form)
        .send()
        .await
        .unwrap();

    // Find entry ID from list page
    let resp = s.client.get(s.url("/content/blog-posts")).send().await.unwrap();
    let body = resp.text().await.unwrap();
    let entry_id = extract_entry_id(&body, "blog-posts").expect("should find entry link");

    // Edit page loads
    let resp = s.client.get(s.url(&format!("/content/blog-posts/{entry_id}/edit")))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // Update
    let form = reqwest::multipart::Form::new()
        .text("title", "Edited Title")
        .text("body", "Updated body");
    let resp = s.client.post(s.url(&format!("/content/blog-posts/{entry_id}")))
        .multipart(form)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::SEE_OTHER);

    // Verify update
    let resp = s.client.get(s.url("/content/blog-posts")).send().await.unwrap();
    let body = resp.text().await.unwrap();
    assert!(body.contains("Edited Title"));

    // Delete
    let resp = s.client.delete(s.url(&format!("/content/blog-posts/{entry_id}")))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);
}

// ── Upload tests ─────────────────────────────────────────────

const GALLERY_SCHEMA: &str = r#"{
    "x-substrukt": {"title": "Gallery", "slug": "gallery", "storage": "directory"},
    "type": "object",
    "properties": {
        "title": {"type": "string", "title": "Title"},
        "image": {"type": "string", "title": "Image", "format": "upload"}
    },
    "required": ["title"]
}"#;

#[tokio::test]
async fn upload_create_and_serve() {
    let s = TestServer::start().await;
    s.setup_admin().await;
    s.create_schema(GALLERY_SCHEMA).await;

    let file_part = reqwest::multipart::Part::bytes(b"fake image data".to_vec())
        .file_name("photo.png")
        .mime_str("image/png")
        .unwrap();
    let form = reqwest::multipart::Form::new()
        .text("title", "My Photo")
        .part("image", file_part);
    let resp = s.client.post(s.url("/content/gallery/new"))
        .multipart(form)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::SEE_OTHER);

    // Get entry to find upload hash
    let resp = s.client.get(s.url("/content/gallery")).send().await.unwrap();
    let body = resp.text().await.unwrap();
    let entry_id = extract_entry_id(&body, "gallery").expect("should find entry");

    let resp = s.client.get(s.url(&format!("/content/gallery/{entry_id}/edit")))
        .send()
        .await
        .unwrap();
    let edit_body = resp.text().await.unwrap();
    assert!(edit_body.contains("Current:"), "Edit should show current upload");

    // Extract upload hash from the edit page link
    if let Some(hash) = extract_upload_hash(&edit_body) {
        let resp = s.client.get(s.url(&format!("/uploads/file/{hash}/photo.png")))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let data = resp.bytes().await.unwrap();
        assert_eq!(&data[..], b"fake image data");
    }
}

#[tokio::test]
async fn upload_dedup() {
    let s = TestServer::start().await;
    s.setup_admin().await;
    s.create_schema(GALLERY_SCHEMA).await;

    // Upload same file twice in different entries
    for title in ["Photo 1", "Photo 2"] {
        let file_part = reqwest::multipart::Part::bytes(b"identical content".to_vec())
            .file_name("img.png")
            .mime_str("image/png")
            .unwrap();
        let form = reqwest::multipart::Form::new()
            .text("title", title.to_string())
            .part("image", file_part);
        s.client.post(s.url("/content/gallery/new"))
            .multipart(form)
            .send()
            .await
            .unwrap();
    }

    // Count upload files on disk — should be 1 (deduplicated)
    let upload_count = std::fs::read_dir(s._data_dir.path().join("uploads"))
        .unwrap()
        .flat_map(|d| std::fs::read_dir(d.unwrap().path()).unwrap())
        .filter(|e| {
            let p = e.as_ref().unwrap().path();
            !p.to_string_lossy().ends_with(".meta.json")
        })
        .count();
    assert_eq!(upload_count, 1, "Same file should be deduplicated");
}

// ── Sidebar nav test ─────────────────────────────────────────

#[tokio::test]
async fn sidebar_shows_content_links() {
    let s = TestServer::start().await;
    s.setup_admin().await;
    s.create_schema(BLOG_SCHEMA).await;

    let resp = s.client.get(s.url("/")).send().await.unwrap();
    let body = resp.text().await.unwrap();
    assert!(body.contains(r#"href="/content/blog-posts""#));
}

// ── Helpers ──────────────────────────────────────────────────

/// Extract the first entry ID from a content list page's edit links.
fn extract_entry_id(html: &str, schema_slug: &str) -> Option<String> {
    let pattern = format!("/content/{schema_slug}/");
    for line in html.lines() {
        if let Some(pos) = line.find(&pattern) {
            let rest = &line[pos + pattern.len()..];
            if let Some(end) = rest.find("/edit") {
                return Some(rest[..end].to_string());
            }
        }
    }
    None
}

/// Extract an upload hash from an edit page's upload link.
fn extract_upload_hash(html: &str) -> Option<String> {
    let marker = "/uploads/file/";
    if let Some(pos) = html.find(marker) {
        let rest = &html[pos + marker.len()..];
        if let Some(end) = rest.find('/') {
            return Some(rest[..end].to_string());
        }
    }
    None
}
