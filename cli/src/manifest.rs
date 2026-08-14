use crate::error::AppError;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub type Selection = BTreeMap<String, String>;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub version: u64,
    pub home: PathBuf,
    pub facets: BTreeMap<String, Facet>,
    #[serde(default)]
    pub files: BTreeMap<String, ManagedFile>,
    #[serde(default)]
    pub effects: BTreeMap<String, Effect>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Facet {
    pub default: String,
    pub variants: BTreeMap<String, PathBuf>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged, deny_unknown_fields)]
pub enum ManagedFile {
    Static(PathBuf),
    Facet { facet: String },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Effect {
    #[serde(default)]
    pub on: Vec<String>,
    pub exec: EffectExec,
}

#[derive(Debug, Deserialize)]
#[serde(untagged, deny_unknown_fields)]
pub enum EffectExec {
    Static(Vec<String>),
    Facet {
        facet: String,
        variants: BTreeMap<String, Vec<String>>,
    },
}

impl Manifest {
    pub fn validate(&self) -> Result<(), AppError> {
        if self.version != 4 {
            return Err(AppError::InvalidManifest(format!(
                "unsupported version {}, expected 4",
                self.version
            )));
        }
        if !self.home.is_absolute() {
            return Err(AppError::InvalidManifest(
                "home must be an absolute path".to_string(),
            ));
        }
        if self.facets.is_empty() {
            return Err(AppError::InvalidManifest(
                "at least one facet is required".to_string(),
            ));
        }

        for (name, facet) in &self.facets {
            if name.is_empty() || name == "." || name == ".." || name.contains('/') {
                return Err(AppError::InvalidManifest(format!(
                    "facet name '{name}' must be one path segment"
                )));
            }
            if facet.variants.is_empty() {
                return Err(AppError::InvalidManifest(format!(
                    "facet '{name}' has no variants"
                )));
            }
            if !facet.variants.contains_key(&facet.default) {
                return Err(AppError::InvalidManifest(format!(
                    "facet '{name}' default '{}' is not a variant",
                    facet.default
                )));
            }
            for (variant, root) in &facet.variants {
                if variant.is_empty() {
                    return Err(AppError::InvalidManifest(format!(
                        "facet '{name}' has an empty variant name"
                    )));
                }
                if !root.is_absolute() {
                    return Err(AppError::InvalidManifest(format!(
                        "root for {name}={variant} must be absolute"
                    )));
                }
            }
        }

        for (path, file) in &self.files {
            if path
                .split('/')
                .any(|segment| segment.is_empty() || segment == "." || segment == "..")
            {
                return Err(AppError::InvalidManifest(format!(
                    "managed file path '{path}' must be a normalized relative path"
                )));
            }

            match file {
                ManagedFile::Static(source) if !source.is_absolute() => {
                    return Err(AppError::InvalidManifest(format!(
                        "static source for '{path}' must be absolute"
                    )));
                }
                ManagedFile::Facet { facet } if !self.facets.contains_key(facet) => {
                    return Err(AppError::InvalidManifest(format!(
                        "file '{path}' references unknown facet '{facet}'"
                    )));
                }
                _ => {}
            }
        }

        for (name, effect) in &self.effects {
            if name.is_empty() {
                return Err(AppError::InvalidManifest(
                    "effect name must not be empty".to_string(),
                ));
            }
            for facet in &effect.on {
                if !self.facets.contains_key(facet) {
                    return Err(AppError::InvalidManifest(format!(
                        "effect '{name}' runs on unknown facet '{facet}'"
                    )));
                }
            }

            match &effect.exec {
                EffectExec::Static(argv) => validate_argv(name, argv)?,
                EffectExec::Facet { facet, variants } => {
                    let definition = self.facets.get(facet).ok_or_else(|| {
                        AppError::InvalidManifest(format!(
                            "effect '{name}' command references unknown facet '{facet}'"
                        ))
                    })?;
                    if effect.on.as_slice() != std::slice::from_ref(facet) {
                        return Err(AppError::InvalidManifest(format!(
                            "effect '{name}' with a facet command must set on = ['{facet}']"
                        )));
                    }
                    if variants.len() != definition.variants.len()
                        || variants
                            .keys()
                            .any(|variant| !definition.variants.contains_key(variant))
                    {
                        return Err(AppError::InvalidManifest(format!(
                            "effect '{name}' command variants do not match facet '{facet}'"
                        )));
                    }
                    for argv in variants.values() {
                        validate_argv(name, argv)?;
                    }
                }
            }
        }

        Ok(())
    }
}

fn validate_argv(name: &str, argv: &[String]) -> Result<(), AppError> {
    let Some(executable) = argv.first() else {
        return Err(AppError::InvalidManifest(format!(
            "effect '{name}' command is empty"
        )));
    };
    if !Path::new(executable).is_absolute() {
        return Err(AppError::InvalidManifest(format!(
            "effect '{name}' executable must be absolute"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_v4_manifest() {
        let manifest: Manifest = serde_json::from_str(
            r#"{
                    "version": 4,
                    "home": "/home/test",
                    "facets": {
                        "theme": {
                            "default": "dark",
                            "variants": {"dark": "/nix/store/dark"}
                        }
                    },
                    "files": {
                        ".config/app": {"facet": "theme"},
                        ".config/static": "/nix/store/static"
                    },
                    "effects": {
                        "always": {
                            "exec": ["/bin/true"]
                        },
                        "reload": {
                            "on": ["theme"],
                            "exec": {
                                "facet": "theme",
                                "variants": {"dark": ["/bin/true"]}
                            }
                        }
                    }
                }"#,
        )
        .unwrap();

        manifest.validate().unwrap();
        assert_eq!(manifest.facets["theme"].default, "dark");
    }

    #[test]
    fn rejects_unsafe_managed_path() {
        for path in ["../outside", "/absolute", "a//b", "a/./b"] {
            let manifest = Manifest {
                version: 4,
                home: PathBuf::from("/home/test"),
                facets: BTreeMap::from([(
                    "theme".to_string(),
                    Facet {
                        default: "dark".to_string(),
                        variants: BTreeMap::from([(
                            "dark".to_string(),
                            PathBuf::from("/nix/store/dark"),
                        )]),
                    },
                )]),
                files: BTreeMap::from([(
                    path.to_string(),
                    ManagedFile::Facet {
                        facet: "theme".to_string(),
                    },
                )]),
                effects: BTreeMap::new(),
            };

            assert!(matches!(
                manifest.validate(),
                Err(AppError::InvalidManifest(_))
            ));
        }
    }
}
