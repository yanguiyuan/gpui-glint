//! Self-hosted plugin repository. Published source is validated as data and never executed.
mod software;
mod web;
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use ed25519_dalek::SigningKey;
use glint_plugin_registry::{
    self as protocol, Index, Info, Package, Publish, Release, Result, SignedIndex,
};
use rusqlite::{Connection, OptionalExtension, params};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Write,
    path::{Path as FsPath, PathBuf},
    sync::{Arc, Mutex},
};
use subtle::ConstantTimeEq;

pub struct Repository {
    connection: Mutex<Connection>,
    root: PathBuf,
    key: SigningKey,
    token_hash: [u8; 32],
}
impl Repository {
    pub fn open(root: &FsPath, token: &str) -> Result<Self> {
        if token.len() < 24 {
            return Err("GLINT_REGISTRY_ADMIN_TOKEN must contain at least 24 characters".into());
        }
        fs::create_dir_all(root.join("packages"))
            .map_err(|_| "Cannot create repository data directory")?;
        let key_path = root.join("signing.key");
        let key = if key_path.exists() {
            let bytes: [u8; 32] = fs::read(&key_path)
                .map_err(|_| "Cannot read signing key")?
                .try_into()
                .map_err(|_| "Invalid signing key")?;
            SigningKey::from_bytes(&bytes)
        } else {
            let bytes = rand::random::<[u8; 32]>();
            let mut options = fs::OpenOptions::new();
            options.create_new(true).write(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut file = options
                .open(&key_path)
                .map_err(|_| "Cannot create signing key")?;
            file.write_all(&bytes)
                .and_then(|_| file.sync_all())
                .map_err(|_| "Cannot save signing key")?;
            SigningKey::from_bytes(&bytes)
        };
        fs::create_dir_all(root.join("software-assets"))
            .map_err(|_| "Cannot create software asset directory")?;
        let connection = Connection::open(root.join("registry.sqlite3"))
            .map_err(|_| "Cannot open registry database")?;
        connection
            .busy_timeout(std::time::Duration::from_secs(10))
            .map_err(|_| "Cannot configure database")?;
        connection.execute_batch("PRAGMA journal_mode=WAL; CREATE TABLE IF NOT EXISTS releases (plugin_id TEXT NOT NULL, version TEXT NOT NULL, metadata TEXT NOT NULL, PRIMARY KEY(plugin_id,version)); CREATE TABLE IF NOT EXISTS state (id INTEGER PRIMARY KEY CHECK(id=1), sequence INTEGER NOT NULL); INSERT OR IGNORE INTO state VALUES(1,0);").map_err(|_| "Cannot initialize registry database")?;
        software::initialize(&connection)?;
        Ok(Self {
            connection: Mutex::new(connection),
            root: root.into(),
            key,
            token_hash: Sha256::digest(token.as_bytes()).into(),
        })
    }
    pub fn info(&self) -> Info {
        protocol::info(&self.key)
    }
    fn authorized(&self, headers: &HeaderMap) -> bool {
        let Some(token) = headers
            .get(header::AUTHORIZATION)
            .and_then(|h| h.to_str().ok())
            .and_then(|h| h.strip_prefix("Bearer "))
        else {
            return false;
        };
        let hash: [u8; 32] = Sha256::digest(token.as_bytes()).into();
        bool::from(self.token_hash.ct_eq(&hash))
    }
    pub fn index(&self) -> Result<SignedIndex> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| "Repository lock failed")?;
        let sequence: i64 = connection
            .query_row("SELECT sequence FROM state WHERE id=1", [], |r| r.get(0))
            .map_err(|_| "Cannot read index sequence")?;
        let mut statement = connection
            .prepare("SELECT metadata FROM releases ORDER BY plugin_id,version")
            .map_err(|_| "Cannot read registry")?;
        let rows = statement
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(|_| "Cannot read registry")?;
        let mut releases = vec![];
        for row in rows {
            releases.push(
                serde_json::from_str(&row.map_err(|_| "Cannot read release")?)
                    .map_err(|_| "Invalid stored release")?,
            );
        }
        let issued_at = protocol::now();
        protocol::sign(
            &Index {
                format: 1,
                source_id: self.info().source_id,
                sequence: sequence.try_into().map_err(|_| "Invalid index sequence")?,
                issued_at,
                expires_at: issued_at + 86400,
                releases,
            },
            &self.key,
        )
    }
    pub fn publish(&self, publish: Publish) -> Result<Release> {
        let bytes = publish.decode()?;
        let package = Package::decode(&bytes)?;
        let release = Release {
            manifest: package.plugin.manifest,
            hosts: publish.hosts,
            sha256: protocol::digest(&bytes),
            size: bytes.len() as u64,
            changelog: publish.changelog,
            yanked: false,
        };
        release.validate()?;
        let mut connection = self
            .connection
            .lock()
            .map_err(|_| "Repository lock failed")?;
        let transaction = connection
            .transaction()
            .map_err(|_| "Cannot begin publication")?;
        let exists: bool = transaction
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM releases WHERE plugin_id=?1 AND version=?2)",
                params![release.manifest.id, release.manifest.version],
                |r| r.get(0),
            )
            .map_err(|_| "Cannot check release")?;
        if exists {
            return Err("Version already exists; published versions are immutable".into());
        }
        let count: i64 = transaction
            .query_row("SELECT COUNT(*) FROM releases", [], |r| r.get(0))
            .map_err(|_| "Cannot count releases")?;
        if count >= protocol::MAX_RELEASES as i64 {
            return Err("Registry release limit reached".into());
        }
        let metadata = serde_json::to_string(&release).map_err(|_| "Cannot encode release")?;
        transaction
            .execute(
                "INSERT INTO releases VALUES(?1,?2,?3)",
                params![release.manifest.id, release.manifest.version, metadata],
            )
            .map_err(|_| "Cannot insert release")?;
        transaction
            .execute("UPDATE state SET sequence=sequence+1 WHERE id=1", [])
            .map_err(|_| "Cannot increment index")?;
        // Check the complete signed index bound before committing publication.
        let total: i64 = transaction
            .query_row(
                "SELECT COALESCE(SUM(LENGTH(CAST(metadata AS BLOB))),0) FROM releases",
                [],
                |r| r.get(0),
            )
            .map_err(|_| "Cannot size index")?;
        if total as usize > protocol::MAX_INDEX - 64 * 1024 {
            return Err("Registry index is full".into());
        }
        let destination = self
            .root
            .join("packages")
            .join(format!("{}.glintpkg", release.sha256));
        if destination.exists() {
            if protocol::digest(
                &fs::read(&destination).map_err(|_| "Cannot verify stored package")?,
            ) != release.sha256
            {
                return Err("Stored package is damaged".into());
            }
        } else {
            let mut staged = tempfile::NamedTempFile::new_in(self.root.join("packages"))
                .map_err(|_| "Cannot stage package")?;
            staged
                .write_all(&bytes)
                .and_then(|_| staged.as_file().sync_all())
                .map_err(|_| "Cannot save package")?;
            staged
                .persist(&destination)
                .map_err(|_| "Cannot commit package")?;
        }
        transaction.commit().map_err(|_| "Cannot commit release")?;
        Ok(release)
    }
    pub fn yank(&self, id: &str, version: &str, yanked: bool) -> Result<Release> {
        let mut connection = self
            .connection
            .lock()
            .map_err(|_| "Repository lock failed")?;
        let transaction = connection
            .transaction()
            .map_err(|_| "Cannot update release")?;
        let text: Option<String> = transaction
            .query_row(
                "SELECT metadata FROM releases WHERE plugin_id=?1 AND version=?2",
                params![id, version],
                |r| r.get(0),
            )
            .optional()
            .map_err(|_| "Cannot read release")?;
        let mut release: Release = serde_json::from_str(&text.ok_or("Release not found")?)
            .map_err(|_| "Invalid release metadata")?;
        if release.yanked != yanked {
            release.yanked = yanked;
            transaction
                .execute(
                    "UPDATE releases SET metadata=?3 WHERE plugin_id=?1 AND version=?2",
                    params![
                        id,
                        version,
                        serde_json::to_string(&release).map_err(|_| "Cannot encode release")?
                    ],
                )
                .map_err(|_| "Cannot update release")?;
            transaction
                .execute("UPDATE state SET sequence=sequence+1 WHERE id=1", [])
                .map_err(|_| "Cannot update index")?;
        }
        transaction
            .commit()
            .map_err(|_| "Cannot commit release update")?;
        Ok(release)
    }
    pub fn package(&self, sha: &str) -> Result<Vec<u8>> {
        if sha.len() != 64
            || !sha
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err("Package not found".into());
        }
        let bytes = fs::read(self.root.join("packages").join(format!("{sha}.glintpkg")))
            .map_err(|_| "Package not found")?;
        if bytes.len() > protocol::MAX_PACKAGE || protocol::digest(&bytes) != sha {
            return Err("Package verification failed".into());
        }
        Ok(bytes)
    }
}
#[derive(Clone)]
struct AppState(Arc<Repository>);
type ApiResult<T> = std::result::Result<T, (StatusCode, Json<serde_json::Value>)>;
fn error(status: StatusCode, message: impl Into<String>) -> (StatusCode, Json<serde_json::Value>) {
    (status, Json(serde_json::json!({"error":message.into()})))
}
async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T> + Send + 'static,
) -> ApiResult<T> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|_| error(StatusCode::INTERNAL_SERVER_ERROR, "Repository task failed"))?
        .map_err(|e| {
            error(
                if e.contains("already exists") {
                    StatusCode::CONFLICT
                } else {
                    StatusCode::BAD_REQUEST
                },
                e,
            )
        })
}
async fn info_handler(State(state): State<AppState>) -> Json<Info> {
    Json(state.0.info())
}
async fn index_handler(State(state): State<AppState>) -> ApiResult<Response> {
    let index = blocking(move || state.0.index()).await?;
    Ok(([(header::CACHE_CONTROL, "no-store")], Json(index)).into_response())
}
async fn publish_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Publish>,
) -> ApiResult<Json<Release>> {
    if !state.0.authorized(&headers) {
        return Err(error(StatusCode::UNAUTHORIZED, "Management token required"));
    }
    blocking(move || state.0.publish(body)).await.map(Json)
}
async fn change_handler(
    state: AppState,
    headers: HeaderMap,
    id: String,
    version: String,
    yanked: bool,
) -> ApiResult<Json<Release>> {
    if !state.0.authorized(&headers) {
        return Err(error(StatusCode::UNAUTHORIZED, "Management token required"));
    }
    blocking(move || state.0.yank(&id, &version, yanked))
        .await
        .map(Json)
}
async fn yank_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((id, version)): Path<(String, String)>,
) -> ApiResult<Json<Release>> {
    change_handler(state, headers, id, version, true).await
}
async fn restore_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((id, version)): Path<(String, String)>,
) -> ApiResult<Json<Release>> {
    change_handler(state, headers, id, version, false).await
}
async fn package_handler(
    State(state): State<AppState>,
    Path(sha): Path<String>,
) -> ApiResult<Response> {
    let bytes = blocking(move || state.0.package(&sha)).await?;
    Ok((
        [
            (header::CONTENT_TYPE, "application/vnd.glint.plugin+json"),
            (header::CACHE_CONTROL, "public, max-age=31536000, immutable"),
        ],
        bytes,
    )
        .into_response())
}
pub fn router(repository: Repository) -> Router {
    Router::new()
        .route("/", get(web::index))
        .route("/catalog.css", get(web::css))
        .route("/catalog.js", get(web::js))
        .route("/glint-api-icon.svg", get(web::icon))
        .route("/skills/glint-plugin-dev/SKILL.md", get(web::skill))
        .route("/v1/apps", get(software::apps))
        .route(
            "/v1/apps/{app}/releases",
            get(software::feed).post(software::publish),
        )
        .route("/v1/apps/{app}/assets/{name}", post(software::upload))
        .route(
            "/v1/apps/{app}/assets/{sha}/{name}",
            get(software::download),
        )
        .route(
            "/v1/apps/{app}/releases/{version}/yank",
            post(software::yank),
        )
        .route(
            "/v1/apps/{app}/releases/{version}/restore",
            post(software::restore),
        )
        .route("/health", get(|| async { "ok" }))
        .route("/v1/info", get(info_handler))
        .route("/v1/index", get(index_handler))
        .route("/v1/publish", post(publish_handler))
        .route("/v1/packages/{sha}", get(package_handler))
        .route("/v1/plugins/{id}/{version}/yank", post(yank_handler))
        .route("/v1/plugins/{id}/{version}/restore", post(restore_handler))
        .layer(DefaultBodyLimit::max(2 * 1024 * 1024))
        .with_state(AppState(Arc::new(repository)))
}
