use crate::error::{error, Context, Error};
use crate::manifest::{Config, NamedSelection, RawSelection, Selection};
use std::fs::{self, OpenOptions};
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub fn switch(
    config: &Config,
    manifest_path: &Path,
    state_dir: &Path,
    sets: &[String],
) -> crate::Result<Selection> {
    fs::create_dir_all(state_dir).context(format_args!(
        "could not create state directory '{}'",
        state_dir.display()
    ))?;
    let lock_path = state_dir.join("switch.lock");
    let lock = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(&lock_path)
        .context(format_args!(
            "could not open switch lock '{}'",
            lock_path.display()
        ))?;
    lock.lock().context(format_args!(
        "could not acquire switch lock '{}'",
        lock_path.display()
    ))?;

    let current = state_dir.join("current");
    let mut selection = match fs::symlink_metadata(&current) {
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => config.default_selection(),
        Err(context) => {
            return Err(Error::context(
                format_args!("could not inspect current pointer '{}'", current.display()),
                context,
            ))
        }
        Ok(metadata) if !metadata.file_type().is_symlink() => {
            return Err(error!("'{}' is not a symlink", current.display()))
        }
        Ok(_) => {
            let path = current.join("selection.json");
            let file = fs::File::open(&path).context(format_args!(
                "could not open current selection '{}'",
                path.display()
            ))?;
            let raw: RawSelection = serde_json::from_reader(file)
                .context(format_args!("could not parse JSON at '{}'", path.display()))?;
            config.parse_selection(raw)?
        }
    };

    let mut requested = Vec::with_capacity(sets.len());
    for set in sets {
        let Some((name, value)) = set.split_once('=') else {
            return Err(error!("'{set}' must have the form facet=value"));
        };
        let facet_id = config
            .facet_id(name)
            .context(format_args!("unknown facet '{name}'"))?;
        let variant_id = config[facet_id]
            .variant_id(value)
            .context(format_args!("unknown value '{value}' for facet '{name}'"))?;
        selection[facet_id] = variant_id;
        requested.push(facet_id);
    }

    for (facet_id, name, facet) in config.facets() {
        let variant_id = selection[facet_id];
        let (value, variant) = facet.variant(variant_id);
        let metadata = fs::metadata(&variant.root).context(format_args!(
            "could not inspect variant root '{}'",
            variant.root.display()
        ))?;
        if !metadata.is_dir() {
            return Err(error!("root for {name}={value} is not a directory"));
        }
    }

    let current_tmp = state_dir.join(".current");
    match fs::remove_file(&current_tmp) {
        Ok(()) => {}
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => {}
        Err(context) => {
            return Err(Error::context(
                format_args!(
                    "could not remove stale current pointer '{}'",
                    current_tmp.display()
                ),
                context,
            ))
        }
    }

    let generations_dir = state_dir.join("generations");
    fs::create_dir_all(&generations_dir).context(format_args!(
        "could not create generations directory '{}'",
        generations_dir.display()
    ))?;
    let generation_id = format!(
        "{}-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos(),
        std::process::id()
    );
    let generation_dir = generations_dir.join(&generation_id);
    fs::create_dir(&generation_dir).context(format_args!(
        "could not create runtime generation '{}'",
        generation_dir.display()
    ))?;
    let mut pending = PendingGeneration {
        path: generation_dir,
        pointer: current_tmp,
        committed: false,
    };

    let root_dir = pending.path.join("root");
    fs::create_dir(&root_dir).context(format_args!(
        "could not create generation roots '{}'",
        root_dir.display()
    ))?;
    let generation_manifest = pending.path.join("manifest");
    symlink(manifest_path, &generation_manifest).context(format_args!(
        "could not link generation manifest '{}'",
        generation_manifest.display()
    ))?;
    let selection_path = pending.path.join("selection.json");
    let selection_file = fs::File::create(&selection_path).context(format_args!(
        "could not create generation selection '{}'",
        selection_path.display()
    ))?;
    serde_json::to_writer_pretty(
        selection_file,
        &NamedSelection {
            config,
            selection: &selection,
        },
    )
    .context("could not serialize selection")?;

    for (facet_id, name, facet) in config.facets() {
        let variant_id = selection[facet_id];
        let variant = facet.variant(variant_id).1;
        let link = root_dir.join(name);
        symlink(&variant.root, &link).context(format_args!(
            "could not link selected variant '{}'",
            link.display()
        ))?;
    }

    symlink(
        PathBuf::from("generations").join(&generation_id),
        &pending.pointer,
    )
    .context(format_args!(
        "could not create current pointer '{}'",
        pending.pointer.display()
    ))?;
    fs::rename(&pending.pointer, &current).context(format_args!(
        "could not replace current pointer '{}'",
        current.display()
    ))?;
    pending.committed = true;

    crate::effects::run(
        config.effects.iter().filter(|effect| {
            effect.on.is_empty() || effect.on.iter().any(|facet| requested.contains(facet))
        }),
        &selection,
    )?;

    Ok(selection)
}

struct PendingGeneration {
    path: PathBuf,
    pointer: PathBuf,
    committed: bool,
}

impl Drop for PendingGeneration {
    fn drop(&mut self) {
        if !self.committed {
            let _ = fs::remove_file(&self.pointer);
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::{Argv, Effect, EffectExec};
    use serde_json::{json, Map, Value};
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::time::{Duration, Instant};

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
        &config[facet_id].variant(variant_id).1.root
    }

    fn script(temp: &Path, name: &str, body: &str) -> PathBuf {
        let path = temp.join(name);
        fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        let mut permissions = fs::metadata(&path).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&path, permissions).unwrap();
        path
    }

    fn command(program: &Path, arguments: &[&str]) -> Argv {
        let mut argv = Vec::with_capacity(arguments.len() + 1);
        argv.push(program.to_string_lossy().into_owned().into_boxed_str());
        argv.extend(arguments.iter().map(|argument| Box::from(*argument)));
        argv.into_boxed_slice()
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
    fn failed_generation_is_removed() {
        let temp = TestDir::new("failed-generation");
        let (mut config, manifest_path, state) = fixture(&temp.0, false);
        let (_, facet) = config.facets.shift_remove_index(0).unwrap();
        config.facets.insert("missing/theme".into(), facet);

        assert!(switch(&config, &manifest_path, &state, &[]).is_err());

        assert_eq!(fs::read_dir(state.join("generations")).unwrap().count(), 0);
        assert!(!state.join(".current").exists());
    }

    #[test]
    fn rejects_invalid_overrides() {
        let temp = TestDir::new("invalid");
        let (config, manifest_path, state) = fixture(&temp.0, false);

        for set in ["unknown=value", "theme=unknown", "theme"] {
            assert!(switch(&config, &manifest_path, &state, &[set.to_string()]).is_err());
        }
        assert!(!state.join("current").exists());
    }

    #[test]
    fn rejects_non_symlink_current_state() {
        let temp = TestDir::new("invalid-state");
        let (config, manifest_path, state) = fixture(&temp.0, false);
        fs::create_dir_all(state.join("current")).unwrap();

        assert!(switch(&config, &manifest_path, &state, &[]).is_err());
    }

    #[test]
    fn runs_only_matching_effects() {
        let temp = TestDir::new("effects");
        let (mut config, manifest_path, state) = fixture(&temp.0, true);
        let log = temp.0.join("effects.log");
        let recorder = script(&temp.0, "record", "printf '%s\\n' \"$1\" >> \"$2\"");
        let theme = config.facet_id("theme").unwrap();
        let density = config.facet_id("density").unwrap();
        let variants = config[theme]
            .variants
            .keys()
            .map(|variant| command(&recorder, &[variant, log.to_str().unwrap()]))
            .collect::<Vec<_>>()
            .into_boxed_slice();
        config.effects = vec![
            Effect {
                name: "always".into(),
                on: Box::default(),
                exec: EffectExec::Static(command(&recorder, &["always", log.to_str().unwrap()])),
            },
            Effect {
                name: "density".into(),
                on: vec![density].into_boxed_slice(),
                exec: EffectExec::Static(command(&recorder, &["density", log.to_str().unwrap()])),
            },
            Effect {
                name: "theme".into(),
                on: vec![theme].into_boxed_slice(),
                exec: EffectExec::Facet {
                    facet: theme,
                    variants,
                },
            },
        ]
        .into_boxed_slice();

        switch(&config, &manifest_path, &state, &[]).unwrap();
        switch(&config, &manifest_path, &state, &["theme=dark".to_string()]).unwrap();

        let effects = fs::read_to_string(log).unwrap();
        assert_eq!(
            effects.lines().filter(|effect| *effect == "always").count(),
            2
        );
        assert_eq!(
            effects.lines().filter(|effect| *effect == "dark").count(),
            1
        );
        assert!(!effects.lines().any(|effect| effect == "density"));
    }

    #[test]
    fn effect_failure_does_not_roll_back_generation() {
        let temp = TestDir::new("effect-failure");
        let (mut config, manifest_path, state) = fixture(&temp.0, false);
        let failure = script(&temp.0, "fail", "printf 'broken' >&2\nexit 7");
        let log = temp.0.join("effects.log");
        let recorder = script(&temp.0, "record-after-failure", ": > \"$1\"");
        let theme = config.facet_id("theme").unwrap();
        config.effects = vec![
            Effect {
                name: "reload".into(),
                on: vec![theme].into_boxed_slice(),
                exec: EffectExec::Static(command(&failure, &[])),
            },
            Effect {
                name: "after".into(),
                on: vec![theme].into_boxed_slice(),
                exec: EffectExec::Static(command(&recorder, &[log.to_str().unwrap()])),
            },
        ]
        .into_boxed_slice();

        let error = switch(
            &config,
            &manifest_path,
            &state,
            &["theme=light".to_string()],
        )
        .unwrap_err()
        .to_string();

        assert!(error.contains("effect 'reload' exited with exit status: 7"));
        assert!(error.contains("stderr:\nbroken"));
        assert!(log.exists());
        assert_eq!(
            serde_json::from_reader::<_, Value>(
                fs::File::open(state.join("current/selection.json")).unwrap()
            )
            .unwrap(),
            json!({"theme": "light"})
        );
    }

    #[test]
    fn effect_timeout_does_not_roll_back_generation() {
        let temp = TestDir::new("effect-timeout");
        let (mut config, manifest_path, state) = fixture(&temp.0, false);
        let timeout = script(&temp.0, "timeout", "sleep 5");
        config.effects = vec![Effect {
            name: "stuck".into(),
            on: Box::default(),
            exec: EffectExec::Static(command(&timeout, &[])),
        }]
        .into_boxed_slice();

        let started = Instant::now();
        let error = switch(&config, &manifest_path, &state, &[])
            .unwrap_err()
            .to_string();

        assert!(error.contains("effect 'stuck' timed out"));
        assert!(started.elapsed() < Duration::from_secs(2));
        assert!(state.join("current/selection.json").exists());
    }

    #[test]
    fn runs_effects_concurrently() {
        let temp = TestDir::new("effect-concurrency");
        let (mut config, manifest_path, state) = fixture(&temp.0, false);
        let barrier = script(
            &temp.0,
            "barrier",
            ": > \"$1\"\nwhile [ ! -e \"$2\" ]; do :; done",
        );
        let first = temp.0.join("first");
        let second = temp.0.join("second");
        config.effects = vec![
            Effect {
                name: "first".into(),
                on: Box::default(),
                exec: EffectExec::Static(command(
                    &barrier,
                    &[first.to_str().unwrap(), second.to_str().unwrap()],
                )),
            },
            Effect {
                name: "second".into(),
                on: Box::default(),
                exec: EffectExec::Static(command(
                    &barrier,
                    &[second.to_str().unwrap(), first.to_str().unwrap()],
                )),
            },
        ]
        .into_boxed_slice();

        switch(&config, &manifest_path, &state, &[]).unwrap();

        assert!(first.exists());
        assert!(second.exists());
    }
}
