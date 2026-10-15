use crate::*;
use fs2::FileExt;
use std::{
    collections::BTreeMap,
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};
#[derive(Default, Serialize, Deserialize)]
struct State {
    sources: Vec<Source>,
    sequences: BTreeMap<String, u64>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Installed {
    pub source: Source,
    pub release: Release,
}
#[derive(Clone)]
pub struct PluginStore {
    root: PathBuf,
}
impl PluginStore {
    pub fn new(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        fs::create_dir_all(&root).map_err(|_| "Cannot create plugin store")?;
        Ok(Self { root })
    }
    fn state<T>(&self, operation: impl FnOnce(&mut State) -> Result<T>) -> Result<T> {
        let lock = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(self.root.join("store.lock"))
            .map_err(|_| "Cannot lock plugin store")?;
        lock.lock_exclusive()
            .map_err(|_| "Cannot lock plugin store")?;
        let path = self.root.join("sources.json");
        let mut state = if path.exists() {
            let bytes = read(&path, 64 * 1024)?;
            serde_json::from_slice(&bytes).map_err(|_| "Invalid plugin source configuration")?
        } else {
            State::default()
        };
        let result = operation(&mut state)?;
        atomic_write(
            &path,
            &serde_json::to_vec_pretty(&state).map_err(|_| "Cannot encode source configuration")?,
        )?;
        Ok(result)
    }
    pub fn sources(&self) -> Result<Vec<Source>> {
        self.state(|s| Ok(s.sources.clone()))
    }
    pub fn source(&self, name: &str) -> Result<Source> {
        self.sources()?
            .into_iter()
            .find(|s| s.name == name)
            .ok_or_else(|| "Plugin source is not configured".into())
    }
    pub fn add_source(&self, source: Source) -> Result<()> {
        source.validate()?;
        let id = source.id()?;
        self.state(|s| {
            if s.sources.len() >= 16
                || s.sources
                    .iter()
                    .any(|old| old.name == source.name || old.id().ok().as_ref() == Some(&id))
            {
                return Err("Duplicate source name/key or source limit reached".into());
            }
            s.sources.push(source);
            Ok(())
        })
    }
    pub fn remove_source(&self, name: &str) -> Result<()> {
        self.state(|s| {
            if !s.sources.iter().any(|source| source.name == name) {
                return Err("Unknown source".into());
            }
            s.sources.retain(|source| source.name != name);
            Ok(())
        })
    }
    pub fn refresh(&self, client: &Client, source: &Source) -> Result<Index> {
        let signed = client.fetch(source)?;
        let source_id = source.id()?;
        let index = self.state(|state| {
            if !state.sources.iter().any(|s| {
                s.name == source.name && s.url == source.url && s.public_key == source.public_key
            }) {
                return Err("Source configuration changed".into());
            }
            let index = verify(
                &signed,
                &source.public_key,
                *state.sequences.get(&source_id).unwrap_or(&0),
                false,
            )?;
            state.sequences.insert(source_id.clone(), index.sequence);
            Ok(index)
        })?;
        atomic_write(
            &self.root.join(format!("cache-{source_id}.json")),
            &serde_json::to_vec(&signed).map_err(|_| "Cannot cache registry")?,
        )?;
        Ok(index)
    }

    pub fn install(
        &self,
        client: &Client,
        source: &Source,
        id: &str,
        version: &str,
        host: &str,
        host_version: &str,
    ) -> Result<Installed> {
        let index = self.refresh(client, source)?;
        let release = index
            .releases
            .into_iter()
            .find(|r| r.manifest.id == id && r.manifest.version == version)
            .ok_or("Release is not in signed index")?;
        if !release.supports(host, host_version) {
            return Err("Plugin is withdrawn or incompatible with this application".into());
        }
        let bytes = client.download(source, &release)?;
        let record = Installed {
            source: source.clone(),
            release,
        };
        let parent = self.root.join("installed").join(source.id()?);
        fs::create_dir_all(&parent).map_err(|_| "Cannot create install directory")?;
        let destination = parent.join(&record.release.sha256);
        if destination.exists() {
            let existing: Installed =
                serde_json::from_slice(&read(&destination.join("installed.json"), MAX_METADATA)?)
                    .map_err(|_| "Invalid existing install")?;
            if existing.release.sha256 != record.release.sha256
                || digest(&read(&destination.join("package.glintpkg"), MAX_PACKAGE)?)
                    != record.release.sha256
            {
                return Err("Existing installation is damaged".into());
            }
            atomic_write(
                &destination.join("installed.json"),
                &serde_json::to_vec_pretty(&record).map_err(|_| "Cannot encode install record")?,
            )?;
            return Ok(record);
        }
        let staging = tempfile::Builder::new()
            .prefix(".staging-")
            .tempdir_in(&parent)
            .map_err(|_| "Cannot stage plugin installation")?;
        fs::write(staging.path().join("package.glintpkg"), &bytes)
            .map_err(|_| "Cannot save plugin package")?;
        fs::write(
            staging.path().join("installed.json"),
            serde_json::to_vec_pretty(&record).map_err(|_| "Cannot encode install record")?,
        )
        .map_err(|_| "Cannot save install record")?;
        fs::rename(staging.path(), &destination)
            .map_err(|_| "Cannot activate plugin installation")?;
        Ok(record)
    }
    pub fn installed(&self) -> Result<Vec<Installed>> {
        let mut records = vec![];
        let root = self.root.join("installed");
        if !root.exists() {
            return Ok(records);
        }
        for source in fs::read_dir(root)
            .map_err(|_| "Cannot list installed plugins")?
            .flatten()
            .take(16)
        {
            if !source.file_type().is_ok_and(|t| t.is_dir()) {
                continue;
            }
            for package in fs::read_dir(source.path())
                .map_err(|_| "Cannot list installed versions")?
                .flatten()
                .take(MAX_RELEASES)
            {
                let Ok(bytes) = read(&package.path().join("installed.json"), MAX_METADATA) else {
                    continue;
                };
                let record: Installed = serde_json::from_slice(&bytes)
                    .map_err(|_| "Invalid installed plugin metadata")?;
                record.source.validate()?;
                record.release.validate()?;
                if source.file_name().to_string_lossy() != record.source.id()?
                    || package.file_name().to_string_lossy() != record.release.sha256
                {
                    return Err("Installed plugin path does not match metadata".into());
                }
                records.push(record);
            }
        }
        records.sort_by(|a, b| {
            (
                &a.source.name,
                &a.release.manifest.id,
                &a.release.manifest.version,
            )
                .cmp(&(
                    &b.source.name,
                    &b.release.manifest.id,
                    &b.release.manifest.version,
                ))
        });
        Ok(records)
    }
    pub fn load(&self, record: &Installed) -> Result<Plugin> {
        record.source.validate()?;
        record.release.validate()?;
        let path = self
            .root
            .join("installed")
            .join(record.source.id()?)
            .join(&record.release.sha256);
        let bytes = read(&path.join("package.glintpkg"), MAX_PACKAGE)?;
        if digest(&bytes) != record.release.sha256 || bytes.len() as u64 != record.release.size {
            return Err("Installed plugin verification failed".into());
        }
        let mut package = Package::decode(&bytes)?;
        if serde_json::to_value(&package.plugin.manifest).ok()
            != serde_json::to_value(&record.release.manifest).ok()
        {
            return Err("Installed plugin metadata mismatch".into());
        }
        package.plugin.origin = Some(glint_plugin::Origin {
            source_id: record.source.id()?,
            package_sha256: record.release.sha256.clone(),
        });
        Ok(package.plugin)
    }
    pub fn uninstall(&self, source: &str, id: &str, version: &str) -> Result<()> {
        let record = self
            .installed()?
            .into_iter()
            .find(|r| {
                r.source.name == source
                    && r.release.manifest.id == id
                    && r.release.manifest.version == version
            })
            .ok_or("Installed plugin not found")?;
        fs::remove_dir_all(
            self.root
                .join("installed")
                .join(record.source.id()?)
                .join(&record.release.sha256),
        )
        .map_err(|_| "Cannot uninstall plugin".into())
    }
}
fn read(path: &Path, limit: usize) -> Result<Vec<u8>> {
    let file = fs::File::open(path).map_err(|_| "Cannot read plugin store file")?;
    let mut bytes = vec![];
    file.take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "Cannot read plugin store file")?;
    if bytes.len() > limit {
        return Err("Plugin store file too large".into());
    }
    Ok(bytes)
}
fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut file = tempfile::NamedTempFile::new_in(path.parent().ok_or("Missing store directory")?)
        .map_err(|_| "Cannot stage plugin store file")?;
    file.write_all(bytes)
        .map_err(|_| "Cannot write plugin store file")?;
    file.as_file()
        .sync_all()
        .map_err(|_| "Cannot sync plugin store file")?;
    file.persist(path)
        .map_err(|_| "Cannot commit plugin store file")?;
    Ok(())
}
