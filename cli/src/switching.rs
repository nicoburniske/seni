use crate::error::AppError;
use crate::manifest::{Config, RawSelection, Selection};
use std::fs::{self, OpenOptions};
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub fn switch(
    config: &Config,
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
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => config.default_selection(),
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
            let raw: RawSelection =
                serde_json::from_reader(file).map_err(|source| AppError::ParseJson {
                    path: path.clone(),
                    source,
                })?;
            config.parse_selection(raw)?
        }
    };

    for set in sets {
        let Some((name, value)) = set.split_once('=') else {
            return Err(AppError::InvalidSelection(format!(
                "'{set}' must have the form facet=value"
            )));
        };
        let facet_id = config
            .facet_id(name)
            .ok_or_else(|| AppError::InvalidSelection(format!("unknown facet '{name}'")))?;
        let variant_id = config[facet_id].variant_id(value).ok_or_else(|| {
            AppError::InvalidSelection(format!("unknown value '{value}' for facet '{name}'"))
        })?;
        selection[facet_id] = variant_id;
    }

    for (facet_id, name, facet) in config.facets() {
        let variant_id = selection[facet_id];
        let (value, variant) = facet.variant(variant_id);
        let metadata = fs::metadata(variant.root())
            .map_err(|source| AppError::fs("inspect variant root", variant.root(), source))?;
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
        serde_json::to_writer_pretty(selection_file, &config.named_selection(&selection))
            .map_err(AppError::SerializeSelection)?;

        for (facet_id, name, facet) in config.facets() {
            let variant_id = selection[facet_id];
            let variant = facet.variant(variant_id).1;
            let link = root_dir.join(name);
            symlink(variant.root(), &link)
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
    use serde_json::{json, Map, Value};
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

    fn fixture(temp: &Path, multiple: bool) -> (Config, PathBuf, PathBuf) {
        let dark = temp.join("dark");
        let light = temp.join("light");
        fs::create_dir(&dark).unwrap();
        fs::create_dir(&light).unwrap();

        let mut facets = Map::new();
        facets.insert(
            "theme".to_string(),
            json!({
                "default": "dark",
                "variants": {"dark": dark, "light": light}
            }),
        );
        if multiple {
            let compact = temp.join("compact");
            let roomy = temp.join("roomy");
            fs::create_dir(&compact).unwrap();
            fs::create_dir(&roomy).unwrap();
            facets.insert(
                "density".to_string(),
                json!({
                    "default": "compact",
                    "variants": {"compact": compact, "roomy": roomy}
                }),
            );
        }

        let encoded = serde_json::to_vec(&json!({
            "version": 4,
            "home": temp,
            "facets": Value::Object(facets)
        }))
        .unwrap();
        let config = Config::parse(encoded.as_slice()).unwrap();
        let manifest_path = temp.join("manifest.json");
        fs::write(&manifest_path, "{}").unwrap();
        (config, manifest_path, temp.join("state"))
    }

    fn selected(config: &Config, selection: &Selection, facet: &str, variant: &str) -> bool {
        let facet_id = config.facet_id(facet).unwrap();
        selection[facet_id] == config[facet_id].variant_id(variant).unwrap()
    }

    fn variant_root<'a>(config: &'a Config, facet: &str, variant: &str) -> &'a Path {
        let facet_id = config.facet_id(facet).unwrap();
        let variant_id = config[facet_id].variant_id(variant).unwrap();
        config[facet_id].variant(variant_id).1.root()
    }

    #[test]
    fn switches_from_defaults() {
        let temp = TestDir::new("defaults");
        let (config, manifest_path, state) = fixture(&temp.0, false);

        let selection = switch(
            &config,
            &manifest_path,
            &state,
            &["theme=light".to_string()],
        )
        .unwrap();

        assert!(selected(&config, &selection, "theme", "light"));
        assert_eq!(
            fs::read_link(state.join("current/root/theme")).unwrap(),
            variant_root(&config, "theme", "light")
        );
        assert_eq!(
            fs::read_link(state.join("current/manifest")).unwrap(),
            manifest_path
        );
        assert_eq!(
            serde_json::from_reader::<_, Value>(
                fs::File::open(state.join("current/selection.json")).unwrap()
            )
            .unwrap(),
            json!({"theme": "light"})
        );
    }

    #[test]
    fn repeated_selection_creates_a_new_generation() {
        let temp = TestDir::new("repeated");
        let (config, manifest_path, state) = fixture(&temp.0, false);
        let set = ["theme=dark".to_string()];

        switch(&config, &manifest_path, &state, &set).unwrap();
        let first = fs::read_link(state.join("current")).unwrap();
        switch(&config, &manifest_path, &state, &set).unwrap();
        let second = fs::read_link(state.join("current")).unwrap();

        assert_ne!(first, second);
    }

    #[test]
    fn switches_multiple_facets_together() {
        let temp = TestDir::new("multiple");
        let (config, manifest_path, state) = fixture(&temp.0, true);

        let selection = switch(
            &config,
            &manifest_path,
            &state,
            &["theme=light".to_string(), "density=roomy".to_string()],
        )
        .unwrap();

        assert!(selected(&config, &selection, "theme", "light"));
        assert!(selected(&config, &selection, "density", "roomy"));
        assert_eq!(
            fs::read_link(state.join("current/root/theme")).unwrap(),
            variant_root(&config, "theme", "light")
        );
        assert_eq!(
            fs::read_link(state.join("current/root/density")).unwrap(),
            variant_root(&config, "density", "roomy")
        );
    }

    #[test]
    fn failed_switch_preserves_current_generation() {
        let temp = TestDir::new("failed");
        let (config, manifest_path, state) = fixture(&temp.0, false);

        switch(&config, &manifest_path, &state, &[]).unwrap();
        let current = fs::read_link(state.join("current")).unwrap();
        fs::remove_dir_all(variant_root(&config, "theme", "light")).unwrap();

        assert!(switch(
            &config,
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
        let (config, manifest_path, state) = fixture(&temp.0, false);

        for set in ["unknown=value", "theme=unknown", "theme"] {
            assert!(matches!(
                switch(&config, &manifest_path, &state, &[set.to_string()]),
                Err(AppError::InvalidSelection(_))
            ));
        }
        assert!(!state.join("current").exists());
    }

    #[test]
    fn rejects_non_symlink_current_state() {
        let temp = TestDir::new("invalid-state");
        let (config, manifest_path, state) = fixture(&temp.0, false);
        fs::create_dir_all(state.join("current")).unwrap();

        assert!(matches!(
            switch(&config, &manifest_path, &state, &[]),
            Err(AppError::InvalidState(_))
        ));
    }
}
