use crate::error::AppError;
use crate::manifest::{Manifest, Selection};
use std::fs::{self, OpenOptions};
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub fn switch(
    manifest: &Manifest,
    manifest_path: &Path,
    state_dir: &Path,
    sets: &[String],
) -> Result<Selection, AppError> {
    fs::create_dir_all(state_dir)
        .map_err(|source| AppError::fs("create state directory", state_dir, source))?;
    let lock_path = state_dir.join("switch.lock");
    let lock = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(&lock_path)
        .map_err(|source| AppError::fs("open switch lock", &lock_path, source))?;
    lock.lock()
        .map_err(|source| AppError::fs("acquire switch lock", &lock_path, source))?;

    let current = state_dir.join("current");
    let mut selection = match fs::symlink_metadata(&current) {
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => manifest
            .facets
            .iter()
            .map(|(name, facet)| (name.clone(), facet.default.clone()))
            .collect(),
        Err(source) => return Err(AppError::fs("inspect current pointer", &current, source)),
        Ok(metadata) if !metadata.file_type().is_symlink() => {
            return Err(AppError::InvalidState(format!(
                "'{}' is not a symlink",
                current.display()
            )))
        }
        Ok(_) => {
            let path = current.join("selection.json");
            let file = fs::File::open(&path)
                .map_err(|source| AppError::fs("open current selection", &path, source))?;
            let selection: Selection =
                serde_json::from_reader(file).map_err(|source| AppError::ParseJson {
                    path: path.clone(),
                    source,
                })?;
            if selection.len() != manifest.facets.len() {
                return Err(AppError::InvalidState(
                    "selection does not contain every manifest facet".to_string(),
                ));
            }
            for (name, value) in &selection {
                let facet = manifest.facets.get(name).ok_or_else(|| {
                    AppError::InvalidState(format!("selection contains unknown facet '{name}'"))
                })?;
                if !facet.variants.contains_key(value) {
                    return Err(AppError::InvalidState(format!(
                        "selection contains invalid value '{value}' for facet '{name}'"
                    )));
                }
            }
            selection
        }
    };

    for set in sets {
        let Some((name, value)) = set.split_once('=') else {
            return Err(AppError::InvalidSelection(format!(
                "'{set}' must have the form facet=value"
            )));
        };
        let facet = manifest
            .facets
            .get(name)
            .ok_or_else(|| AppError::InvalidSelection(format!("unknown facet '{name}'")))?;
        if !facet.variants.contains_key(value) {
            return Err(AppError::InvalidSelection(format!(
                "unknown value '{value}' for facet '{name}'"
            )));
        }
        selection.insert(name.to_string(), value.to_string());
    }

    for (name, value) in &selection {
        let root = &manifest.facets[name].variants[value];
        let metadata = fs::metadata(root)
            .map_err(|source| AppError::fs("inspect variant root", root, source))?;
        if !metadata.is_dir() {
            return Err(AppError::InvalidManifest(format!(
                "root for {name}={value} is not a directory"
            )));
        }
    }

    let current_tmp = state_dir.join(".current");
    match fs::remove_file(&current_tmp) {
        Ok(()) => {}
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => {}
        Err(source) => {
            return Err(AppError::fs(
                "remove stale current pointer",
                &current_tmp,
                source,
            ))
        }
    }

    let generations_dir = state_dir.join("generations");
    fs::create_dir_all(&generations_dir)
        .map_err(|source| AppError::fs("create generations directory", &generations_dir, source))?;
    let generation_id = format!(
        "{}-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos(),
        std::process::id()
    );
    let generation_dir = generations_dir.join(&generation_id);
    fs::create_dir(&generation_dir)
        .map_err(|source| AppError::fs("create runtime generation", &generation_dir, source))?;

    let build_result = (|| {
        let root_dir = generation_dir.join("root");
        fs::create_dir(&root_dir)
            .map_err(|source| AppError::fs("create generation roots", &root_dir, source))?;
        let generation_manifest = generation_dir.join("manifest");
        symlink(manifest_path, &generation_manifest).map_err(|source| {
            AppError::fs("link generation manifest", &generation_manifest, source)
        })?;
        let selection_path = generation_dir.join("selection.json");
        let selection_file = fs::File::create(&selection_path).map_err(|source| {
            AppError::fs("create generation selection", &selection_path, source)
        })?;
        serde_json::to_writer_pretty(selection_file, &selection)
            .map_err(AppError::SerializeSelection)?;

        for (name, value) in &selection {
            let root = &manifest.facets[name].variants[value];
            let link = root_dir.join(name);
            symlink(root, &link)
                .map_err(|source| AppError::fs("link selected variant", &link, source))?;
        }

        Ok::<_, AppError>(())
    })();
    if let Err(error) = build_result {
        let _ = fs::remove_dir_all(&generation_dir);
        return Err(error);
    }

    if let Err(source) = symlink(
        PathBuf::from("generations").join(&generation_id),
        &current_tmp,
    ) {
        let _ = fs::remove_dir_all(&generation_dir);
        return Err(AppError::fs("create current pointer", &current_tmp, source));
    }
    if let Err(source) = fs::rename(&current_tmp, &current) {
        let _ = fs::remove_file(&current_tmp);
        let _ = fs::remove_dir_all(&generation_dir);
        return Err(AppError::fs("replace current pointer", &current, source));
    }

    Ok(selection)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::Facet;
    use std::collections::BTreeMap;
    use std::fs;

    struct TestDir(PathBuf);

    impl TestDir {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!("sumi-{}-{name}", std::process::id()));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn fixture(temp: &Path) -> (Manifest, PathBuf, PathBuf) {
        let dark = temp.join("dark");
        let light = temp.join("light");
        fs::create_dir(&dark).unwrap();
        fs::create_dir(&light).unwrap();
        let manifest_path = temp.join("manifest.json");
        fs::write(&manifest_path, "{}").unwrap();
        let manifest = Manifest {
            version: 4,
            home: temp.to_path_buf(),
            facets: BTreeMap::from([(
                "theme".to_string(),
                Facet {
                    default: "dark".to_string(),
                    variants: BTreeMap::from([
                        ("dark".to_string(), dark),
                        ("light".to_string(), light),
                    ]),
                },
            )]),
            files: BTreeMap::new(),
            effects: BTreeMap::new(),
        };
        let state = temp.join("state");
        (manifest, manifest_path, state)
    }

    #[test]
    fn switches_from_defaults() {
        let temp = TestDir::new("defaults");
        let (manifest, manifest_path, state) = fixture(&temp.0);

        let selection = switch(
            &manifest,
            &manifest_path,
            &state,
            &["theme=light".to_string()],
        )
        .unwrap();

        assert_eq!(selection["theme"], "light");
        assert_eq!(
            fs::read_link(state.join("current/root/theme")).unwrap(),
            manifest.facets["theme"].variants["light"]
        );
        assert_eq!(
            fs::read_link(state.join("current/manifest")).unwrap(),
            manifest_path
        );
    }

    #[test]
    fn repeated_selection_creates_a_new_generation() {
        let temp = TestDir::new("repeated");
        let (manifest, manifest_path, state) = fixture(&temp.0);
        let set = ["theme=dark".to_string()];

        switch(&manifest, &manifest_path, &state, &set).unwrap();
        let first = fs::read_link(state.join("current")).unwrap();
        switch(&manifest, &manifest_path, &state, &set).unwrap();
        let second = fs::read_link(state.join("current")).unwrap();

        assert_ne!(first, second);
    }

    #[test]
    fn switches_multiple_facets_together() {
        let temp = TestDir::new("multiple");
        let (mut manifest, manifest_path, state) = fixture(&temp.0);
        let compact = temp.0.join("compact");
        let roomy = temp.0.join("roomy");
        fs::create_dir(&compact).unwrap();
        fs::create_dir(&roomy).unwrap();
        manifest.facets.insert(
            "density".to_string(),
            Facet {
                default: "compact".to_string(),
                variants: BTreeMap::from([
                    ("compact".to_string(), compact),
                    ("roomy".to_string(), roomy),
                ]),
            },
        );

        switch(
            &manifest,
            &manifest_path,
            &state,
            &["theme=light".to_string(), "density=roomy".to_string()],
        )
        .unwrap();

        assert_eq!(
            fs::read_link(state.join("current/root/theme")).unwrap(),
            manifest.facets["theme"].variants["light"]
        );
        assert_eq!(
            fs::read_link(state.join("current/root/density")).unwrap(),
            manifest.facets["density"].variants["roomy"]
        );
    }

    #[test]
    fn failed_switch_preserves_current_generation() {
        let temp = TestDir::new("failed");
        let (manifest, manifest_path, state) = fixture(&temp.0);

        switch(&manifest, &manifest_path, &state, &[]).unwrap();
        let current = fs::read_link(state.join("current")).unwrap();
        fs::remove_dir_all(&manifest.facets["theme"].variants["light"]).unwrap();

        assert!(switch(
            &manifest,
            &manifest_path,
            &state,
            &["theme=light".to_string()]
        )
        .is_err());
        assert_eq!(fs::read_link(state.join("current")).unwrap(), current);
    }

    #[test]
    fn rejects_invalid_overrides() {
        let temp = TestDir::new("invalid");
        let (manifest, manifest_path, state) = fixture(&temp.0);

        for set in ["unknown=value", "theme=unknown", "theme"] {
            assert!(matches!(
                switch(&manifest, &manifest_path, &state, &[set.to_string()]),
                Err(AppError::InvalidSelection(_))
            ));
        }
        assert!(!state.join("current").exists());
    }

    #[test]
    fn rejects_non_symlink_current_state() {
        let temp = TestDir::new("invalid-state");
        let (manifest, manifest_path, state) = fixture(&temp.0);
        fs::create_dir_all(state.join("current")).unwrap();

        assert!(matches!(
            switch(&manifest, &manifest_path, &state, &[]),
            Err(AppError::InvalidState(_))
        ));
    }
}
