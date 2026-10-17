//! Immutable software artifacts and signed feeds, sharing the source's persistent signing identity.
use super::*;
use axum::{
    body::{Body, Bytes},
    extract::Query,
};
use futures_util::StreamExt;
use glint_plugin_registry::software as spec;
use std::collections::HashMap;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
pub(super) fn initialize(connection: &Connection) -> Result<()> {
    connection.execute_batch("CREATE TABLE IF NOT EXISTS software_releases(app TEXT NOT NULL,version TEXT NOT NULL,metadata TEXT NOT NULL,PRIMARY KEY(app,version)); CREATE TABLE IF NOT EXISTS software_state(app TEXT PRIMARY KEY,sequence INTEGER NOT NULL);").map_err(|_|"Cannot initialize software releases".into())
}
fn valid_app(app: &str) -> Result<()> {
    if !spec::valid_app(app) {
        Err("Invalid application ID".into())
    } else {
        Ok(())
    }
}
impl Repository {
    fn software_releases(&self, app: &str) -> Result<(u64, Vec<spec::Release>)> {
        valid_app(app)?;
        let connection = self
            .connection
            .lock()
            .map_err(|_| "Repository lock failed")?;
        let sequence = connection
            .query_row(
                "SELECT sequence FROM software_state WHERE app=?1",
                [app],
                |r| r.get::<_, i64>(0),
            )
            .optional()
            .map_err(|_| "Cannot read software sequence")?
            .unwrap_or(0);
        let mut statement = connection
            .prepare("SELECT metadata FROM software_releases WHERE app=?1 ORDER BY version")
            .map_err(|_| "Cannot read software releases")?;
        let rows = statement
            .query_map([app], |row| row.get::<_, String>(0))
            .map_err(|_| "Cannot read software releases")?;
        let releases = rows
            .map(|r| {
                serde_json::from_str(&r.map_err(|_| "Cannot read software release")?)
                    .map_err(|_| "Invalid stored software release".to_string())
            })
            .collect::<Result<Vec<_>>>()?;
        Ok((
            sequence
                .try_into()
                .map_err(|_| "Invalid software sequence")?,
            releases,
        ))
    }
    fn software_feed(&self, app: &str) -> Result<SignedIndex> {
        let (sequence, releases) = self.software_releases(app)?;
        let time = protocol::now();
        spec::sign(
            &spec::Feed {
                format: 1,
                app_id: app.into(),
                source_id: self.info().source_id,
                sequence,
                issued_at: time,
                expires_at: time + 86400,
                releases,
            },
            &self.key,
        )
    }
    fn publish_software(&self, app: &str, mut release: spec::Release) -> Result<spec::Release> {
        valid_app(app)?;
        release.published_at = protocol::now();
        release.yanked = false;
        release.validate()?;
        for artifact in &release.artifacts {
            let path = self.root.join("software-assets").join(&artifact.sha256);
            let mut file =
                fs::File::open(path).map_err(|_| "Upload the artifact before publishing")?;
            if file
                .metadata()
                .map_err(|_| "Cannot inspect artifact")?
                .len()
                != artifact.size
            {
                return Err("Uploaded artifact size mismatch".into());
            }
            use std::io::Read;
            let mut hash = Sha256::new();
            let mut buffer = [0u8; 65536];
            loop {
                let n = file
                    .read(&mut buffer)
                    .map_err(|_| "Cannot verify uploaded artifact")?;
                if n == 0 {
                    break;
                }
                hash.update(&buffer[..n]);
            }
            if format!("{:x}", hash.finalize()) != artifact.sha256 {
                return Err("Uploaded artifact digest mismatch".into());
            }
        }
        let mut connection = self
            .connection
            .lock()
            .map_err(|_| "Repository lock failed")?;
        let tx = connection
            .transaction()
            .map_err(|_| "Cannot begin software publication")?;
        let exists: bool = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM software_releases WHERE app=?1 AND version=?2)",
                params![app, release.version],
                |r| r.get(0),
            )
            .map_err(|_| "Cannot check software version")?;
        if exists {
            return Err("Version already exists; software releases are immutable".into());
        }
        let count: i64 = tx
            .query_row(
                "SELECT COUNT(*) FROM software_releases WHERE app=?1",
                [app],
                |r| r.get(0),
            )
            .map_err(|_| "Cannot count software versions")?;
        if count >= spec::MAX_VERSIONS as i64 {
            return Err("Software version limit reached".into());
        }
        let apps: i64 = tx
            .query_row("SELECT COUNT(*) FROM software_state", [], |r| r.get(0))
            .map_err(|_| "Cannot count software applications")?;
        let known: bool = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM software_state WHERE app=?1)",
                [app],
                |r| r.get(0),
            )
            .map_err(|_| "Cannot inspect applications")?;
        if !known && apps >= 32 {
            return Err("Software application limit reached".into());
        }
        let metadata =
            serde_json::to_string(&release).map_err(|_| "Cannot encode software release")?;
        tx.execute(
            "INSERT INTO software_releases VALUES(?1,?2,?3)",
            params![app, release.version, metadata],
        )
        .map_err(|_| "Cannot publish software version")?;
        let size: i64 = tx
            .query_row(
                "SELECT SUM(LENGTH(CAST(metadata AS BLOB))) FROM software_releases WHERE app=?1",
                [app],
                |r| r.get(0),
            )
            .map_err(|_| "Cannot size software index")?;
        if size > protocol::MAX_INDEX as i64 - 65536 {
            return Err("Software feed is full".into());
        }
        tx.execute("INSERT INTO software_state VALUES(?1,1) ON CONFLICT(app) DO UPDATE SET sequence=sequence+1",[app]).map_err(|_|"Cannot advance software sequence")?;
        tx.commit()
            .map_err(|_| "Cannot commit software publication")?;
        Ok(release)
    }
    fn software_yank(&self, app: &str, version: &str, yanked: bool) -> Result<spec::Release> {
        valid_app(app)?;
        let mut connection = self
            .connection
            .lock()
            .map_err(|_| "Repository lock failed")?;
        let tx = connection
            .transaction()
            .map_err(|_| "Cannot begin software update")?;
        let metadata: String = tx
            .query_row(
                "SELECT metadata FROM software_releases WHERE app=?1 AND version=?2",
                params![app, version],
                |r| r.get(0),
            )
            .map_err(|_| "Software release not found")?;
        let mut release: spec::Release =
            serde_json::from_str(&metadata).map_err(|_| "Invalid stored software release")?;
        if release.yanked != yanked {
            release.yanked = yanked;
            tx.execute(
                "UPDATE software_releases SET metadata=?3 WHERE app=?1 AND version=?2",
                params![
                    app,
                    version,
                    serde_json::to_string(&release)
                        .map_err(|_| "Cannot encode software release")?
                ],
            )
            .map_err(|_| "Cannot update software release")?;
            tx.execute(
                "UPDATE software_state SET sequence=sequence+1 WHERE app=?1",
                [app],
            )
            .map_err(|_| "Cannot advance software sequence")?;
        }
        tx.commit().map_err(|_| "Cannot commit software update")?;
        Ok(release)
    }
}
pub(super) async fn apps(State(state): State<AppState>) -> ApiResult<Json<Vec<String>>> {
    blocking(move || {
        let c = state
            .0
            .connection
            .lock()
            .map_err(|_| "Repository lock failed")?;
        let mut s = c
            .prepare("SELECT app FROM software_state ORDER BY app")
            .map_err(|_| "Cannot list software")?;
        let rows = s
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(|_| "Cannot list software")?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|_| "Cannot list software".into())
    })
    .await
    .map(Json)
}
pub(super) async fn feed(
    State(state): State<AppState>,
    Path(app): Path<String>,
) -> ApiResult<Response> {
    let signed = blocking(move || state.0.software_feed(&app)).await?;
    Ok(([(header::CACHE_CONTROL, "no-store")], Json(signed)).into_response())
}
pub(super) async fn publish(
    State(state): State<AppState>,
    Path(app): Path<String>,
    headers: HeaderMap,
    Json(release): Json<spec::Release>,
) -> ApiResult<Json<spec::Release>> {
    if !state.0.authorized(&headers) {
        return Err(error(StatusCode::UNAUTHORIZED, "Management token required"));
    }
    blocking(move || state.0.publish_software(&app, release))
        .await
        .map(Json)
}
async fn change(
    state: AppState,
    app: String,
    version: String,
    headers: HeaderMap,
    yanked: bool,
) -> ApiResult<Json<spec::Release>> {
    if !state.0.authorized(&headers) {
        return Err(error(StatusCode::UNAUTHORIZED, "Management token required"));
    }
    blocking(move || state.0.software_yank(&app, &version, yanked))
        .await
        .map(Json)
}
pub(super) async fn yank(
    State(state): State<AppState>,
    Path((app, version)): Path<(String, String)>,
    headers: HeaderMap,
) -> ApiResult<Json<spec::Release>> {
    change(state, app, version, headers, true).await
}
pub(super) async fn restore(
    State(state): State<AppState>,
    Path((app, version)): Path<(String, String)>,
    headers: HeaderMap,
) -> ApiResult<Json<spec::Release>> {
    change(state, app, version, headers, false).await
}
pub(super) async fn upload(
    State(state): State<AppState>,
    Path((app, name)): Path<(String, String)>,
    Query(query): Query<HashMap<String, String>>,
    headers: HeaderMap,
    body: Body,
) -> ApiResult<Json<spec::Artifact>> {
    if !state.0.authorized(&headers) {
        return Err(error(StatusCode::UNAUTHORIZED, "Management token required"));
    }
    let target = query.get("target").cloned().unwrap_or_default();
    if !spec::valid_app(&app)
        || !spec::valid_name(&name)
        || !spec::valid_name(&target)
        || target.len() > 64
    {
        return Err(error(
            StatusCode::BAD_REQUEST,
            "Invalid software artifact path",
        ));
    }
    let declared = headers
        .get(header::CONTENT_LENGTH)
        .map(|h| {
            h.to_str()
                .ok()
                .and_then(|s| s.parse::<u64>().ok())
                .ok_or_else(|| error(StatusCode::BAD_REQUEST, "Invalid content length"))
        })
        .transpose()?;
    if declared.is_some_and(|s| s == 0 || s > spec::MAX_ARTIFACT) {
        return Err(error(
            StatusCode::PAYLOAD_TOO_LARGE,
            "Artifact exceeds size limit",
        ));
    }
    let temporary =
        tempfile::NamedTempFile::new_in(state.0.root.join("software-assets")).map_err(|_| {
            error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Cannot stage software artifact",
            )
        })?;
    let mut file = tokio::fs::File::from_std(temporary.reopen().map_err(|_| {
        error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Cannot stage software artifact",
        )
    })?);
    let mut stream = body.into_data_stream();
    let mut size = 0;
    let mut hash = Sha256::new();
    while let Some(chunk) = stream.next().await {
        let chunk =
            chunk.map_err(|_| error(StatusCode::BAD_REQUEST, "Software upload interrupted"))?;
        size += chunk.len() as u64;
        if size > spec::MAX_ARTIFACT {
            return Err(error(
                StatusCode::PAYLOAD_TOO_LARGE,
                "Artifact exceeds size limit",
            ));
        }
        hash.update(&chunk);
        file.write_all(&chunk).await.map_err(|_| {
            error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Cannot write software artifact",
            )
        })?;
    }
    if size == 0 || declared.is_some_and(|n| n != size) {
        return Err(error(
            StatusCode::BAD_REQUEST,
            "Software upload size mismatch",
        ));
    }
    file.sync_all().await.map_err(|_| {
        error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Cannot save software artifact",
        )
    })?;
    drop(file);
    let sha256 = format!("{:x}", hash.finalize());
    let destination = state.0.root.join("software-assets").join(&sha256);
    if !destination.exists() {
        temporary.persist_noclobber(destination).map_err(|_| {
            error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Cannot commit software artifact",
            )
        })?;
    }
    Ok(Json(spec::Artifact {
        target,
        name,
        size,
        sha256,
    }))
}
pub(super) async fn download(
    State(state): State<AppState>,
    Path((app, sha, name)): Path<(String, String, String)>,
) -> ApiResult<Response> {
    if !spec::valid_app(&app)
        || !spec::valid_name(&name)
        || sha.len() != 64
        || !sha
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(error(StatusCode::NOT_FOUND, "Software artifact not found"));
    }
    let app_copy = app.clone();
    let sha_copy = sha.clone();
    let name_copy = name.clone();
    let repo = state.0.clone();
    let artifact = blocking(move || {
        repo.software_releases(&app_copy)?
            .1
            .iter()
            .flat_map(|r| r.artifacts.iter())
            .find(|a| a.sha256 == sha_copy && a.name == name_copy)
            .cloned()
            .ok_or("Software artifact not found".into())
    })
    .await?;
    let file = tokio::fs::File::open(state.0.root.join("software-assets").join(&sha))
        .await
        .map_err(|_| error(StatusCode::NOT_FOUND, "Software artifact not found"))?;
    if file
        .metadata()
        .await
        .map_err(|_| {
            error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Cannot read software artifact",
            )
        })?
        .len()
        != artifact.size
    {
        return Err(error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Software artifact size mismatch",
        ));
    }
    let stream = futures_util::stream::try_unfold(file, |mut file| async move {
        let mut buf = vec![0u8; 65536];
        let n = file.read(&mut buf).await?;
        if n == 0 {
            Ok::<_, std::io::Error>(None)
        } else {
            buf.truncate(n);
            Ok(Some((Bytes::from(buf), file)))
        }
    });
    let mut response = Body::from_stream(stream).into_response();
    let h = response.headers_mut();
    h.insert(
        header::CONTENT_TYPE,
        "application/octet-stream".parse().unwrap(),
    );
    h.insert(header::CONTENT_LENGTH, artifact.size.into());
    h.insert(
        header::CACHE_CONTROL,
        "public, max-age=31536000, immutable".parse().unwrap(),
    );
    h.insert(
        header::CONTENT_DISPOSITION,
        format!("attachment; filename=\"{name}\"").parse().unwrap(),
    );
    Ok(response)
}
