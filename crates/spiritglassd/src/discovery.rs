use anyhow::{Context, Result, ensure};
use spiritglass_protocol::*;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Read,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    time::SystemTime,
};

pub const MAX_MODULES: usize = 32;

#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    pub manifest: ModuleManifest,
    pub path: Option<PathBuf>,
    pub enforce_id: bool,
}
impl Config {
    pub fn legacy(executable: PathBuf, args: Vec<String>) -> Self {
        let executable = std::path::absolute(&executable).unwrap_or(executable);
        Self {
            manifest: ModuleManifest {
                id: "local".into(),
                name: "Local module".into(),
                executable,
                args,
                working_directory: None,
                environment: BTreeMap::new(),
                lifecycle: LifecyclePolicy::Always,
                enabled: true,
                watch: vec![],
                stale_after_ms: None,
            },
            path: None,
            enforce_id: false,
        }
    }
    pub fn fingerprint(&self) -> Result<Vec<FileStamp>> {
        std::iter::once(&self.manifest.executable)
            .chain(&self.manifest.watch)
            .map(|path| {
                let metadata = fs::metadata(path)
                    .with_context(|| format!("read deployed file {}", path.display()))?;
                ensure!(
                    metadata.is_file(),
                    "deployed path is not a file: {}",
                    path.display()
                );
                Ok(FileStamp {
                    path: path.clone(),
                    modified: metadata.modified()?,
                    length: metadata.len(),
                    inode: metadata.ino(),
                    mode: metadata.mode(),
                })
            })
            .collect()
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct FileStamp {
    path: PathBuf,
    modified: SystemTime,
    length: u64,
    inode: u64,
    mode: u32,
}

pub struct Discovery {
    pub configs: BTreeMap<String, Config>,
    pub present: BTreeSet<PathBuf>,
    pub errors: Vec<String>,
}

pub fn scan(directory: &Path) -> Result<Discovery> {
    let mut discovery = Discovery {
        configs: BTreeMap::new(),
        present: BTreeSet::new(),
        errors: vec![],
    };
    let mut paths = Vec::new();
    for entry in fs::read_dir(directory).with_context(|| format!("scan {}", directory.display()))? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            let path = path.join("module.json");
            if path.exists() {
                paths.push(path);
            }
        } else if path.extension().is_some_and(|value| value == "json") {
            paths.push(path);
        }
        ensure!(
            paths.len() <= MAX_MODULES,
            "module directory exceeds limit of {MAX_MODULES} manifests"
        );
    }
    paths.sort();
    ensure!(
        paths.len() <= MAX_MODULES,
        "module directory exceeds limit of {MAX_MODULES} manifests"
    );
    let mut duplicates = BTreeSet::new();
    for path in paths {
        discovery.present.insert(path.clone());
        match load(&path) {
            Ok(config) => {
                let id = config.manifest.id.clone();
                if discovery.configs.insert(id.clone(), config).is_some() {
                    duplicates.insert(id);
                }
            }
            Err(error) => discovery
                .errors
                .push(format!("{}: {error:#}", path.display())),
        }
    }
    for id in duplicates {
        discovery.configs.remove(&id);
        discovery.errors.push(format!(
            "duplicate module ID {id}; keeping any running instance unchanged"
        ));
    }
    Ok(discovery)
}

fn load(path: &Path) -> Result<Config> {
    ensure!(
        fs::metadata(path)?.is_file(),
        "manifest is not a regular file"
    );
    let mut bytes = Vec::new();
    fs::File::open(path)?.take(16_385).read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= 16_384, "manifest exceeds 16 KiB");
    let mut manifest: ModuleManifest = serde_json::from_slice(&bytes)?;
    manifest.validate()?;
    let base = path.parent().context("manifest has no parent")?;
    let resolve = |value: &Path| {
        if value.is_absolute() {
            value.to_owned()
        } else {
            base.join(value)
        }
    };
    manifest.executable = resolve(&manifest.executable);
    manifest.working_directory = manifest.working_directory.as_deref().map(resolve);
    manifest.watch = manifest.watch.iter().map(|path| resolve(path)).collect();
    Ok(Config {
        manifest,
        path: Some(path.to_owned()),
        enforce_id: true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn resolves_paths_and_reports_invalid_and_duplicate_manifests() {
        let dir = tempfile::tempdir().unwrap();
        let good = serde_json::json!({"id":"sample", "name":"Sample", "executable":"bin/module", "watch":["settings.json"]});
        fs::write(dir.path().join("a.json"), good.to_string()).unwrap();
        let scan = scan(dir.path()).unwrap();
        assert_eq!(
            scan.configs["sample"].manifest.executable,
            dir.path().join("bin/module")
        );
        fs::write(dir.path().join("b.json"), good.to_string()).unwrap();
        fs::write(dir.path().join("bad.json"), "{bad").unwrap();
        let scan = super::scan(dir.path()).unwrap();
        assert!(scan.configs.is_empty());
        assert_eq!(scan.errors.len(), 2);
        assert_eq!(scan.present.len(), 3);
    }

    #[test]
    fn rejects_oversized_and_non_regular_manifests() {
        let dir = tempfile::tempdir_in("/tmp").unwrap();
        fs::write(dir.path().join("huge.json"), vec![b' '; 32_768]).unwrap();
        let _socket =
            std::os::unix::net::UnixListener::bind(dir.path().join("socket.json")).unwrap();
        let discovery = scan(dir.path()).unwrap();
        assert!(discovery.configs.is_empty());
        assert_eq!(discovery.errors.len(), 2);
        assert!(
            discovery
                .errors
                .iter()
                .any(|error| error.contains("16 KiB"))
        );
        assert!(
            discovery
                .errors
                .iter()
                .any(|error| error.contains("regular file"))
        );
    }
}
