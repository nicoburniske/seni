use crate::error::{error, Context, Error};
use crate::manifest::{
    Config, ExistingFileStrategy, ManagedFile, NamedSelection, RawSelection, Selection, Source,
};
use std::borrow::Cow;
use std::fs::{self, OpenOptions};
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};

pub fn activate(
    config: &Config,
    manifest_path: &Path,
    state_dir: &Path,
) -> crate::Result<Selection> {
    let state = LockedState::open(state_dir)?;
    let mut selection = config.default_selection();
    let previous = if let Some(raw) = state.current_selection()? {
        for (facet_id, name, facet) in config.facets() {
            if let Some(variant_id) = raw
                .get(name)
                .and_then(|value| facet.variant_id(value.as_ref()))
            {
                selection[facet_id] = variant_id;
            }
        }

        state.current_config()?
    } else {
        None
    };

    let mut pending = state.build(config, manifest_path, &selection)?;

    if let Some(previous) = &previous {
        for file in &previous.files {
            if previous.home == config.home
                && config.files.iter().any(|current| current.path == file.path)
            {
                continue;
            }

            let target = previous.home.join(file.path.as_ref());
            if path_points_to(&target, &managed_source(previous, state_dir, file))? {
                fs::remove_file(&target)
                    .context(format_args!("stale managed target '{}'", target.display()))?;
            }
        }
    }

    for file in &config.files {
        let target = config.home.join(file.path.as_ref());
        let source = managed_source(config, state_dir, file);
        if path_points_to(&target, &source)? {
            continue;
        }
        fs::create_dir_all(target.parent().unwrap())
            .context(format_args!("parent directory for '{}'", target.display()))?;
        let temporary = TemporaryLink::create(&source, &target)?;
        match config.existing_file_strategy {
            ExistingFileStrategy::Fail => {
                if !owned_by_previous(previous.as_ref(), config, state_dir, file, &target)?
                    && path_metadata(&target, "managed target")?.is_some()
                {
                    return Err(error!(
                        "managed target '{}' already exists",
                        target.display()
                    ));
                }
                replace_target(&temporary, &target)?;
            }
            ExistingFileStrategy::Clobber => {
                if path_metadata(&target, "managed target")?
                    .is_some_and(|metadata| metadata.file_type().is_dir())
                {
                    fs::remove_dir_all(&target)
                        .context(format_args!("managed target '{}'", target.display()))?;
                }
                replace_target(&temporary, &target)?;
            }
            ExistingFileStrategy::Backup => {
                if owned_by_previous(previous.as_ref(), config, state_dir, file, &target)?
                    || path_metadata(&target, "managed target")?.is_none()
                {
                    replace_target(&temporary, &target)?;
                } else {
                    let backup = target.with_added_extension("seni-backup");
                    if path_metadata(&backup, "backup target")?.is_some() {
                        return Err(error!(
                            "backup target '{}' already exists",
                            backup.display()
                        ));
                    }
                    fs::rename(&target, &backup)
                        .context(format_args!("backup target '{}'", target.display()))?;
                    replace_target(&temporary, &target)?;
                    eprintln!(
                        "seni: backed up '{}' to '{}'",
                        target.display(),
                        backup.display()
                    );
                }
            }
        }
    }

    pending.commit(state_dir)?;
    crate::effects::run(config.effects.iter(), &selection)?;

    Ok(selection)
}

pub fn deactivate(state_dir: &Path) -> crate::Result<Deactivation> {
    match fs::symlink_metadata(state_dir) {
        Ok(_) => {}
        Err(context) if context.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Deactivation::default())
        }
        Err(context) => {
            return Err(Error::context(
                format_args!("state directory '{}'", state_dir.display()),
                context,
            ))
        }
    }
    let state = LockedState::open(state_dir)?;
    let Some(config) = state.current_config()? else {
        state.clear()?;
        return Ok(Deactivation::default());
    };

    let mut summary = Deactivation::default();
    for file in &config.files {
        let target = config.home.join(file.path.as_ref());
        let metadata = match fs::symlink_metadata(&target) {
            Ok(metadata) => metadata,
            Err(context) if context.kind() == std::io::ErrorKind::NotFound => {
                summary.missing += 1;
                continue;
            }
            Err(context) => {
                eprintln!(
                    "seni: warning: managed target '{}': {context}",
                    target.display()
                );
                summary.failed += 1;
                continue;
            }
        };
        if !metadata.file_type().is_symlink() {
            summary.changed += 1;
            continue;
        }
        let actual = match fs::read_link(&target) {
            Ok(actual) => actual,
            Err(context) => {
                eprintln!(
                    "seni: warning: managed target '{}': {context}",
                    target.display()
                );
                summary.failed += 1;
                continue;
            }
        };
        if actual != managed_source(&config, state_dir, file) {
            summary.changed += 1;
            continue;
        }
        match fs::remove_file(&target) {
            Ok(()) => summary.removed += 1,
            Err(context) => {
                eprintln!(
                    "seni: warning: managed target '{}': {context}",
                    target.display()
                );
                summary.failed += 1;
            }
        }
    }

    if summary.failed == 0 {
        state.clear()?;
    }
    Ok(summary)
}

pub fn switch(
    config: &Config,
    manifest_path: &Path,
    state_dir: &Path,
    sets: &[String],
) -> crate::Result<Selection> {
    let state = LockedState::open(state_dir)?;
    let raw = state
        .current_selection()?
        .context("configuration is not active. run 'seni activate'")?;
    let mut selection = config.parse_selection(raw)?;

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

    let mut pending = state.build(config, manifest_path, &selection)?;
    pending.commit(state_dir)?;

    crate::effects::run(
        config.effects.iter().filter(|effect| {
            effect.on.is_empty() || effect.on.iter().any(|facet| requested.contains(facet))
        }),
        &selection,
    )?;

    Ok(selection)
}

pub fn current_selection(config: &Config, state_dir: &Path) -> crate::Result<Selection> {
    let state = LockedState::open(state_dir)?;
    match state.current_selection()? {
        Some(raw) => config.parse_selection(raw),
        None => Ok(config.default_selection()),
    }
}

#[derive(Default)]
pub struct Deactivation {
    pub removed: usize,
    pub missing: usize,
    pub changed: usize,
    pub failed: usize,
}

fn managed_source<'a>(
    config: &'a Config,
    state_dir: &Path,
    file: &'a ManagedFile,
) -> Cow<'a, Path> {
    match &file.source {
        Source::Static(source) => Cow::Borrowed(source),
        Source::Facet(facet_id) => Cow::Owned(
            state_dir
                .join("current/root")
                .join(
                    config
                        .facets()
                        .find(|(candidate, _, _)| candidate == facet_id)
                        .unwrap()
                        .1,
                )
                .join(file.path.as_ref()),
        ),
    }
}

fn path_metadata(path: &Path, description: &str) -> crate::Result<Option<fs::Metadata>> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => Ok(Some(metadata)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(Error::context(
            format_args!("{description} '{}'", path.display()),
            error,
        )),
    }
}

fn path_points_to(path: &Path, expected: &Path) -> crate::Result<bool> {
    let Some(metadata) = path_metadata(path, "managed target")? else {
        return Ok(false);
    };
    if !metadata.file_type().is_symlink() {
        return Ok(false);
    }
    Ok(
        fs::read_link(path).context(format_args!("managed target '{}'", path.display()))?
            == expected,
    )
}

fn owned_by_previous(
    previous: Option<&Config>,
    config: &Config,
    state_dir: &Path,
    file: &ManagedFile,
    target: &Path,
) -> crate::Result<bool> {
    let Some(previous) = previous.filter(|previous| previous.home == config.home) else {
        return Ok(false);
    };
    let Some(file) = previous
        .files
        .iter()
        .find(|previous| previous.path == file.path)
    else {
        return Ok(false);
    };
    path_points_to(target, &managed_source(previous, state_dir, file))
}

struct TemporaryLink(PathBuf);

impl TemporaryLink {
    fn create(source: &Path, target: &Path) -> crate::Result<Self> {
        let pid = std::process::id();
        let mut attempt = 0;
        // retry beside the target so stale candidates cannot block an atomic rename
        loop {
            let candidate = target.with_added_extension(format!("seni-tmp-{pid}-{attempt}"));
            match symlink(source, &candidate) {
                Ok(()) => return Ok(Self(candidate)),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => attempt += 1,
                Err(error) => {
                    return Err(Error::context(
                        format_args!("temporary link '{}'", candidate.display()),
                        error,
                    ))
                }
            }
        }
    }
}

impl Drop for TemporaryLink {
    fn drop(&mut self) {
        if let Err(error) = fs::remove_file(&self.0) {
            if error.kind() != std::io::ErrorKind::NotFound {
                eprintln!(
                    "seni: warning: temporary link '{}': {error}",
                    self.0.display()
                );
            }
        }
    }
}

#[track_caller]
fn replace_target(temporary: &TemporaryLink, target: &Path) -> crate::Result<()> {
    fs::rename(&temporary.0, target).context(format_args!("managed link '{}'", target.display()))
}

struct LockedState<'a> {
    dir: &'a Path,
    current: bool,
    inactive: &'static str,
    _lock: fs::File,
}

impl<'a> LockedState<'a> {
    fn open(dir: &'a Path) -> crate::Result<Self> {
        fs::create_dir_all(dir).context(format_args!("state directory '{}'", dir.display()))?;
        let lock_path = dir.join("switch.lock");
        let lock = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(false)
            .open(&lock_path)
            .context(format_args!("state lock '{}'", lock_path.display()))?;
        lock.lock()
            .context(format_args!("state lock '{}'", lock_path.display()))?;

        let current = dir.join("current");
        let (current_exists, inactive) = match fs::symlink_metadata(&current) {
            Err(context) if context.kind() == std::io::ErrorKind::NotFound => (false, "0"),
            Err(context) => {
                return Err(Error::context(
                    format_args!("current pointer '{}'", current.display()),
                    context,
                ))
            }
            Ok(metadata) if !metadata.file_type().is_symlink() => {
                return Err(error!("'{}' is not a symlink", current.display()))
            }
            Ok(_) => {
                let target = fs::read_link(&current)
                    .context(format_args!("current pointer '{}'", current.display()))?;
                let inactive = match target.as_path() {
                    path if path == Path::new("states/0") => "1",
                    path if path == Path::new("states/1") => "0",
                    _ => {
                        return Err(error!(
                            "current pointer '{}' has invalid target '{}'",
                            current.display(),
                            target.display()
                        ))
                    }
                };
                (true, inactive)
            }
        };

        Ok(Self {
            dir,
            current: current_exists,
            inactive,
            _lock: lock,
        })
    }

    fn current_selection(&self) -> crate::Result<Option<RawSelection>> {
        if !self.current {
            return Ok(None);
        }
        let path = self.dir.join("current/selection.json");
        let file = fs::File::open(&path)
            .context(format_args!("current selection '{}'", path.display()))?;
        Ok(Some(serde_json::from_reader(file).context(
            format_args!("selection JSON '{}'", path.display()),
        )?))
    }

    fn current_config(&self) -> crate::Result<Option<Config>> {
        if !self.current {
            return Ok(None);
        }
        let path = self.dir.join("current/manifest");
        let file =
            fs::File::open(&path).context(format_args!("current manifest '{}'", path.display()))?;
        Ok(Some(Config::parse(file).context(format_args!(
            "current manifest '{}'",
            path.display()
        ))?))
    }

    fn clear(&self) -> crate::Result<()> {
        if self.current {
            let current = self.dir.join("current");
            fs::remove_file(&current)
                .context(format_args!("current pointer '{}'", current.display()))?;
        }
        let states = self.dir.join("states");
        match fs::remove_dir_all(&states) {
            Ok(()) => Ok(()),
            Err(context) if context.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(context) => Err(Error::context(
                format_args!("runtime states '{}'", states.display()),
                context,
            )),
        }
    }

    fn build(
        &self,
        config: &Config,
        manifest_path: &Path,
        selection: &Selection,
    ) -> crate::Result<PendingState> {
        for (facet_id, name, facet) in config.facets() {
            let variant_id = selection[facet_id];
            let (value, variant) = facet.variant(variant_id);
            let metadata = fs::metadata(&variant.root)
                .context(format_args!("variant root '{}'", variant.root.display()))?;
            if !metadata.is_dir() {
                return Err(error!("root for {name}={value} is not a directory"));
            }
        }
        for file in &config.files {
            let source = match &file.source {
                Source::Static(source) => Cow::Borrowed(source.as_path()),
                Source::Facet(facet_id) => {
                    let facet = &config[*facet_id];
                    Cow::Owned(
                        facet
                            .variant(selection[*facet_id])
                            .1
                            .root
                            .join(file.path.as_ref()),
                    )
                }
            };
            fs::metadata(&source).context(format_args!(
                "source '{}' for '{}'",
                source.display(),
                file.path
            ))?;
        }

        let pointer = self.dir.join(".current");
        match fs::remove_file(&pointer) {
            Ok(()) => {}
            Err(context) if context.kind() == std::io::ErrorKind::NotFound => {}
            Err(context) => {
                return Err(Error::context(
                    format_args!("pending pointer '{}'", pointer.display()),
                    context,
                ))
            }
        }

        let states_dir = self.dir.join("states");
        fs::create_dir_all(&states_dir)
            .context(format_args!("states directory '{}'", states_dir.display()))?;
        let state = states_dir.join(self.inactive);
        match fs::remove_dir_all(&state) {
            Ok(()) => {}
            Err(context) if context.kind() == std::io::ErrorKind::NotFound => {}
            Err(context) => {
                return Err(Error::context(
                    format_args!("inactive state '{}'", state.display()),
                    context,
                ))
            }
        }
        fs::create_dir(&state).context(format_args!("runtime state '{}'", state.display()))?;
        let pending = PendingState {
            path: state,
            pointer,
            target: PathBuf::from("states").join(self.inactive),
            committed: false,
        };

        let root_dir = pending.path.join("root");
        fs::create_dir(&root_dir).context(format_args!("state roots '{}'", root_dir.display()))?;
        let state_manifest = pending.path.join("manifest");
        symlink(manifest_path, &state_manifest).context(format_args!(
            "state manifest '{}'",
            state_manifest.display()
        ))?;
        let selection_path = pending.path.join("selection.json");
        let selection_file = fs::File::create(&selection_path).context(format_args!(
            "state selection '{}'",
            selection_path.display()
        ))?;
        serde_json::to_writer_pretty(selection_file, &NamedSelection { config, selection })
            .context("selection JSON")?;

        for (facet_id, name, facet) in config.facets() {
            let variant = facet.variant(selection[facet_id]).1;
            let link = root_dir.join(name);
            symlink(&variant.root, &link)
                .context(format_args!("variant link '{}'", link.display()))?;
        }

        Ok(pending)
    }
}

struct PendingState {
    path: PathBuf,
    pointer: PathBuf,
    target: PathBuf,
    committed: bool,
}

impl PendingState {
    fn commit(&mut self, state_dir: &Path) -> crate::Result<()> {
        symlink(&self.target, &self.pointer)
            .context(format_args!("pending pointer '{}'", self.pointer.display()))?;
        let current = state_dir.join("current");
        fs::rename(&self.pointer, &current)
            .context(format_args!("current pointer '{}'", current.display()))?;
        self.committed = true;
        Ok(())
    }
}

impl Drop for PendingState {
    fn drop(&mut self) {
        if !self.committed {
            if let Err(error) = fs::remove_file(&self.pointer) {
                if error.kind() != std::io::ErrorKind::NotFound {
                    eprintln!(
                        "seni: warning: pending pointer '{}': {error}",
                        self.pointer.display()
                    );
                }
            }
            if let Err(error) = fs::remove_dir_all(&self.path) {
                if error.kind() != std::io::ErrorKind::NotFound {
                    eprintln!(
                        "seni: warning: pending state '{}': {error}",
                        self.path.display()
                    );
                }
            }
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
            let path = std::env::temp_dir().join(format!("seni-{}-{name}", std::process::id()));
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

    fn manifest(temp: &Path, name: &str, multiple: bool, files: Value) -> (Config, PathBuf) {
        let dark = temp.join("dark");
        let light = temp.join("light");
        fs::create_dir_all(&dark).unwrap();
        fs::create_dir_all(&light).unwrap();

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
            fs::create_dir_all(&compact).unwrap();
            fs::create_dir_all(&roomy).unwrap();
            facets.insert(
                "density".to_string(),
                json!({
                    "default": "compact",
                    "variants": {"compact": compact, "roomy": roomy}
                }),
            );
        }

        let encoded = serde_json::to_vec(&json!({
            "version": 1,
            "home": temp,
            "existingFileStrategy": "fail",
            "facets": Value::Object(facets),
            "files": files
        }))
        .unwrap();
        let config = Config::parse(encoded.as_slice()).unwrap();
        let manifest_path = temp.join(name);
        fs::write(&manifest_path, encoded).unwrap();
        (config, manifest_path)
    }

    fn fixture(temp: &Path, multiple: bool) -> (Config, PathBuf, PathBuf) {
        let (config, manifest_path) = manifest(temp, "manifest.json", multiple, json!({}));
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
    fn deactivation_does_not_create_missing_state() {
        let temp = TestDir::new("deactivate-missing");
        let state = temp.0.join("state");
        let summary = deactivate(&state).unwrap();

        assert_eq!(
            (
                summary.removed,
                summary.missing,
                summary.changed,
                summary.failed
            ),
            (0, 0, 0, 0)
        );
        assert_eq!(
            fs::symlink_metadata(state).unwrap_err().kind(),
            std::io::ErrorKind::NotFound
        );
    }

    #[test]
    fn handles_existing_files_by_configured_strategy() {
        let temp = TestDir::new("existing-files");

        for (name, strategy) in [
            ("fail", ExistingFileStrategy::Fail),
            ("clobber", ExistingFileStrategy::Clobber),
            ("backup", ExistingFileStrategy::Backup),
        ] {
            let home = temp.0.join(name);
            let source = temp.0.join(format!("{name}-source"));
            fs::write(&source, "managed").unwrap();
            let (mut config, manifest_path) =
                manifest(&home, "manifest.json", false, json!({"target": source}));
            config.existing_file_strategy = strategy;
            let target = home.join("target");
            if strategy == ExistingFileStrategy::Clobber {
                fs::create_dir(&target).unwrap();
                fs::write(target.join("original"), "original").unwrap();
            } else {
                fs::write(&target, "original").unwrap();
            }

            if strategy == ExistingFileStrategy::Backup {
                let backup = target.with_added_extension("seni-backup");
                fs::write(&backup, "older").unwrap();
                assert!(activate(&config, &manifest_path, &home.join("state")).is_err());
                assert!(!target
                    .with_added_extension(format!("seni-tmp-{}-0", std::process::id()))
                    .is_symlink());
                fs::remove_file(backup).unwrap();
            }

            let result = activate(&config, &manifest_path, &home.join("state"));
            match strategy {
                ExistingFileStrategy::Fail => {
                    assert!(result.unwrap_err().to_string().contains("already exists"));
                    assert_eq!(fs::read_to_string(&target).unwrap(), "original");
                }
                ExistingFileStrategy::Clobber => {
                    result.unwrap();
                    assert_eq!(fs::read_link(&target).unwrap(), source);
                }
                ExistingFileStrategy::Backup => {
                    result.unwrap();
                    assert_eq!(fs::read_link(&target).unwrap(), source);
                    assert_eq!(
                        fs::read_to_string(target.with_added_extension("seni-backup")).unwrap(),
                        "original"
                    );
                }
            }
        }
    }

    #[test]
    fn switches_selections_between_state_directories() {
        let temp = TestDir::new("switch");
        let (config, manifest_path, state) = fixture(&temp.0, true);
        let set = ["theme=light".to_string(), "density=roomy".to_string()];

        activate(&config, &manifest_path, &state).unwrap();
        let selection = switch(&config, &manifest_path, &state, &set).unwrap();
        let first = fs::read_link(state.join("current")).unwrap();

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
        assert_eq!(
            fs::read_link(state.join("current/manifest")).unwrap(),
            manifest_path
        );
        assert_eq!(
            serde_json::from_reader::<_, Value>(
                fs::File::open(state.join("current/selection.json")).unwrap()
            )
            .unwrap(),
            json!({"theme": "light", "density": "roomy"})
        );
        for set in ["unknown=value", "theme=unknown", "theme"] {
            assert!(switch(&config, &manifest_path, &state, &[set.to_string()]).is_err());
            assert_eq!(fs::read_link(state.join("current")).unwrap(), first);
        }
        switch(&config, &manifest_path, &state, &set).unwrap();
        let second = fs::read_link(state.join("current")).unwrap();
        switch(&config, &manifest_path, &state, &set).unwrap();
        let third = fs::read_link(state.join("current")).unwrap();

        assert_ne!(first, second);
        assert_eq!(first, third);
        assert_eq!(fs::read_dir(state.join("states")).unwrap().count(), 2);
    }

    #[test]
    fn failed_switch_preserves_current_and_removes_pending_state() {
        let temp = TestDir::new("failed-switch");
        let (mut config, manifest_path, state) = fixture(&temp.0, false);
        activate(&config, &manifest_path, &state).unwrap();
        let current = fs::read_link(state.join("current")).unwrap();
        let (_, facet) = config.facets.shift_remove_index(0).unwrap();
        config.facets.insert("missing/theme".into(), facet);
        fs::write(
            state.join("current/selection.json"),
            r#"{"missing/theme":"dark"}"#,
        )
        .unwrap();

        assert!(switch(&config, &manifest_path, &state, &[]).is_err());

        assert_eq!(fs::read_link(state.join("current")).unwrap(), current);
        assert_eq!(fs::read_dir(state.join("states")).unwrap().count(), 1);
        assert!(!state.join(".current").exists());
    }

    #[test]
    fn activation_reconciles_configuration() {
        let temp = TestDir::new("activate");
        let dynamic = ".config/app/theme";
        fs::create_dir_all(temp.0.join("dark/.config/app")).unwrap();
        fs::create_dir_all(temp.0.join("light/.config/app")).unwrap();
        fs::write(temp.0.join("dark").join(dynamic), "dark").unwrap();
        fs::write(temp.0.join("light").join(dynamic), "light").unwrap();
        let old = temp.0.join("old");
        let removed = temp.0.join("removed");
        let new = temp.0.join("new");
        let added = temp.0.join("added");
        fs::write(&old, "old").unwrap();
        fs::write(&removed, "removed").unwrap();
        fs::write(&new, "new").unwrap();
        fs::write(&added, "added").unwrap();
        let (mut old_config, old_manifest) = manifest(
            &temp.0,
            "old.json",
            true,
            json!({
                ".config/app/current": old,
                ".config/app/removed": removed,
                ".config/app/theme": {"facet": "theme"}
            }),
        );
        let (mut new_config, new_manifest) = manifest(
            &temp.0,
            "new.json",
            true,
            json!({
                ".config/app/current": new,
                ".config/app/added": added,
                ".config/app/theme": {"facet": "theme"}
            }),
        );
        let state = temp.0.join("state");
        let temporary = temp.0.join(format!(
            ".config/app/current.seni-tmp-{}-0",
            std::process::id()
        ));
        fs::create_dir_all(temporary.parent().unwrap()).unwrap();
        symlink(&old, &temporary).unwrap();
        activate(&old_config, &old_manifest, &state).unwrap();
        assert_eq!(fs::read_link(&temporary).unwrap(), old);

        let log = temp.0.join("effects.log");
        let recorder = script(&temp.0, "record", "printf '%s\\n' \"$1\" >> \"$2\"");
        let theme = old_config.facet_id("theme").unwrap();
        let density = old_config.facet_id("density").unwrap();
        let variants = old_config[theme]
            .variants
            .keys()
            .map(|variant| command(&recorder, &[variant, log.to_str().unwrap()]))
            .collect::<Vec<_>>()
            .into_boxed_slice();
        old_config.effects = vec![
            Effect {
                name: "always".into(),
                on: Box::default(),
                exec: EffectExec::Static(command(&recorder, &["always", log.to_str().unwrap()])),
                ignore_failure: false,
            },
            Effect {
                name: "density".into(),
                on: vec![density].into_boxed_slice(),
                exec: EffectExec::Static(command(&recorder, &["density", log.to_str().unwrap()])),
                ignore_failure: false,
            },
            Effect {
                name: "theme".into(),
                on: vec![theme].into_boxed_slice(),
                exec: EffectExec::Facet {
                    facet: theme,
                    variants,
                },
                ignore_failure: false,
            },
        ]
        .into_boxed_slice();
        switch(
            &old_config,
            &old_manifest,
            &state,
            &["theme=light".to_string()],
        )
        .unwrap();
        let effects = fs::read_to_string(&log).unwrap();
        assert!(effects.lines().any(|effect| effect == "always"));
        assert!(effects.lines().any(|effect| effect == "light"));
        assert!(!effects.lines().any(|effect| effect == "density"));

        let density = new_config.facet_id("density").unwrap();
        new_config.effects = vec![Effect {
            name: "activation".into(),
            on: vec![density].into_boxed_slice(),
            exec: EffectExec::Static(command(&recorder, &["activation", log.to_str().unwrap()])),
            ignore_failure: false,
        }]
        .into_boxed_slice();
        let selection = activate(&new_config, &new_manifest, &state).unwrap();

        assert!(selected(&new_config, &selection, "theme", "light"));
        assert_eq!(
            fs::read_link(temp.0.join(".config/app/current")).unwrap(),
            new
        );
        assert_eq!(
            fs::read_link(temp.0.join(".config/app/added")).unwrap(),
            added
        );
        assert_eq!(
            fs::symlink_metadata(temp.0.join(".config/app/removed"))
                .unwrap_err()
                .kind(),
            std::io::ErrorKind::NotFound
        );
        let target = temp.0.join(dynamic);
        assert_eq!(
            fs::read_link(&target).unwrap(),
            state.join("current/root/theme").join(dynamic)
        );
        assert_eq!(fs::read_to_string(&target).unwrap(), "light");
        assert!(fs::read_to_string(&log)
            .unwrap()
            .lines()
            .any(|effect| effect == "activation"));

        let summary = deactivate(&state).unwrap();
        assert_eq!(
            (
                summary.removed,
                summary.missing,
                summary.changed,
                summary.failed
            ),
            (3, 0, 0, 0)
        );
        for path in ["current", "added", "theme"] {
            assert_eq!(
                fs::symlink_metadata(temp.0.join(".config/app").join(path))
                    .unwrap_err()
                    .kind(),
                std::io::ErrorKind::NotFound
            );
        }
        assert!(!state.join("current").exists());
        assert!(!state.join("states").exists());
        assert!(switch(&new_config, &new_manifest, &state, &[]).is_err());

        let selection = activate(&new_config, &new_manifest, &state).unwrap();
        assert!(selected(&new_config, &selection, "theme", "dark"));
        assert!(selected(&new_config, &selection, "density", "compact"));
        assert_eq!(fs::read_to_string(&target).unwrap(), "dark");

        fs::write(
            state.join("current/selection.json"),
            r#"{"theme":"missing"}"#,
        )
        .unwrap();
        let selection = activate(&new_config, &new_manifest, &state).unwrap();
        assert!(selected(&new_config, &selection, "theme", "dark"));
        assert!(selected(&new_config, &selection, "density", "compact"));

        let managed = temp.0.join(".config/app/current");
        fs::remove_file(&managed).unwrap();
        fs::write(&managed, "mine").unwrap();
        let current = fs::read_link(state.join("current")).unwrap();
        let error = activate(&new_config, &new_manifest, &state)
            .unwrap_err()
            .to_string();
        assert!(error.contains("already exists"));
        assert_eq!(fs::read_to_string(&managed).unwrap(), "mine");
        assert_eq!(fs::read_link(state.join("current")).unwrap(), current);
        assert!(!state.join(".current").exists());

        let summary = deactivate(&state).unwrap();
        assert_eq!(
            (
                summary.removed,
                summary.missing,
                summary.changed,
                summary.failed
            ),
            (2, 0, 1, 0)
        );
        assert_eq!(fs::read_to_string(&managed).unwrap(), "mine");
        assert!(!state.join("current").exists());
        assert!(!state.join("states").exists());
    }

    #[test]
    fn effect_failures_finish_concurrently_without_rolling_back_state() {
        let temp = TestDir::new("effect-failures");
        let (mut config, manifest_path, state) = fixture(&temp.0, false);
        activate(&config, &manifest_path, &state).unwrap();
        let failure = script(&temp.0, "fail", "printf 'broken' >&2\nexit 7");
        let timeout = script(&temp.0, "timeout", "sleep 5");
        let barrier = script(
            &temp.0,
            "barrier",
            ": > \"$1\"\nwhile [ ! -e \"$2\" ]; do :; done",
        );
        let first = temp.0.join("first");
        let second = temp.0.join("second");
        let theme = config.facet_id("theme").unwrap();
        config.effects = vec![
            Effect {
                name: "failure".into(),
                on: vec![theme].into_boxed_slice(),
                exec: EffectExec::Static(command(&failure, &[])),
                ignore_failure: true,
            },
            Effect {
                name: "timeout".into(),
                on: vec![theme].into_boxed_slice(),
                exec: EffectExec::Static(command(&timeout, &[])),
                ignore_failure: false,
            },
            Effect {
                name: "first".into(),
                on: vec![theme].into_boxed_slice(),
                exec: EffectExec::Static(command(
                    &barrier,
                    &[first.to_str().unwrap(), second.to_str().unwrap()],
                )),
                ignore_failure: false,
            },
            Effect {
                name: "second".into(),
                on: vec![theme].into_boxed_slice(),
                exec: EffectExec::Static(command(
                    &barrier,
                    &[second.to_str().unwrap(), first.to_str().unwrap()],
                )),
                ignore_failure: false,
            },
        ]
        .into_boxed_slice();

        let started = Instant::now();
        let error = switch(
            &config,
            &manifest_path,
            &state,
            &["theme=light".to_string()],
        )
        .unwrap_err()
        .to_string();

        assert!(!error.contains("effect 'failure'"));
        assert!(error.contains("effect 'timeout' timed out"));
        assert!(started.elapsed() < Duration::from_secs(2));
        assert!(first.exists());
        assert!(second.exists());
        assert_eq!(
            serde_json::from_reader::<_, Value>(
                fs::File::open(state.join("current/selection.json")).unwrap()
            )
            .unwrap(),
            json!({"theme": "light"})
        );
    }
}
