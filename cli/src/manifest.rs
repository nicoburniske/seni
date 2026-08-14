use crate::error::{error, Context};
use indexmap::IndexMap;
use serde::ser::{SerializeMap, Serializer};
use serde::{Deserialize, Serialize};
use std::io::Read;
use std::ops::{Index, IndexMut};
use std::path::{Path, PathBuf};

pub type Argv = Box<[Box<str>]>;
pub type RawSelection = IndexMap<Box<str>, Box<str>>;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct FacetId(usize);

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct VariantId(usize);

#[derive(Debug)]
pub struct Config {
    pub home: PathBuf,
    pub facets: IndexMap<Box<str>, Facet>,
    pub files: Box<[ManagedFile]>,
    pub effects: Box<[Effect]>,
}

#[derive(Debug)]
pub struct Facet {
    pub default: VariantId,
    pub variants: IndexMap<Box<str>, Variant>,
}

#[derive(Debug)]
pub struct Variant {
    pub root: PathBuf,
}

#[derive(Debug)]
pub struct ManagedFile {
    pub path: Box<str>,
    pub source: Source,
}

#[derive(Debug)]
pub enum Source {
    Static(PathBuf),
    Facet(FacetId),
}

#[derive(Debug)]
pub struct Effect {
    pub name: Box<str>,
    pub on: Box<[FacetId]>,
    pub exec: EffectExec,
    pub ignore_failure: bool,
}

#[derive(Debug)]
pub enum EffectExec {
    Static(Argv),
    Facet {
        facet: FacetId,
        variants: Box<[Argv]>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Selection(Box<[VariantId]>);

impl Config {
    pub fn parse(reader: impl Read) -> crate::Result<Self> {
        let raw: RawManifest =
            serde_json::from_reader(reader).context("could not parse manifest JSON")?;
        if raw.version != 4 {
            return Err(error!("unsupported version {}, expected 4", raw.version));
        }
        if !raw.home.is_absolute() {
            return Err(error!("home must be an absolute path"));
        }
        if raw.facets.is_empty() {
            return Err(error!("at least one facet is required"));
        }

        let mut facets = IndexMap::with_capacity(raw.facets.len());
        for (name, raw_facet) in raw.facets {
            if name.is_empty()
                || name.as_ref() == "."
                || name.as_ref() == ".."
                || name.contains('/')
            {
                return Err(error!("facet name '{name}' must be one path segment"));
            }
            if raw_facet.variants.is_empty() {
                return Err(error!("facet '{name}' has no variants"));
            }

            let mut variants = IndexMap::with_capacity(raw_facet.variants.len());
            for (variant_name, root) in raw_facet.variants {
                if variant_name.is_empty() {
                    return Err(error!("facet '{name}' has an empty variant name"));
                }
                if !root.is_absolute() {
                    return Err(error!("root for {name}={variant_name} must be absolute"));
                }
                variants.insert(variant_name, Variant { root });
            }
            let default = variants
                .get_index_of(raw_facet.default.as_ref())
                .map(VariantId)
                .context(format_args!(
                    "facet '{name}' default '{}' is not a variant",
                    raw_facet.default
                ))?;
            facets.insert(name, Facet { default, variants });
        }

        let mut files = Vec::with_capacity(raw.files.len());
        for (path, raw_file) in raw.files {
            if path
                .split('/')
                .any(|segment| segment.is_empty() || segment == "." || segment == "..")
            {
                return Err(error!(
                    "managed file path '{path}' must be a normalized relative path"
                ));
            }
            let source =
                match raw_file {
                    RawManagedFile::Static(source) => {
                        if !source.is_absolute() {
                            return Err(error!("static source for '{path}' must be absolute"));
                        }
                        Source::Static(source)
                    }
                    RawManagedFile::Facet { facet } => {
                        let facet_id = facets.get_index_of(facet.as_ref()).map(FacetId).context(
                            format_args!("file '{path}' references unknown facet '{facet}'"),
                        )?;
                        Source::Facet(facet_id)
                    }
                };
            files.push(ManagedFile { path, source });
        }

        let mut effects = Vec::with_capacity(raw.effects.len());
        for (name, raw_effect) in raw.effects {
            if name.is_empty() {
                return Err(error!("effect name must not be empty"));
            }

            let mut on = Vec::with_capacity(raw_effect.on.len());
            for facet in raw_effect.on {
                let facet_id =
                    facets
                        .get_index_of(facet.as_ref())
                        .map(FacetId)
                        .context(format_args!(
                            "effect '{name}' runs on unknown facet '{facet}'"
                        ))?;
                on.push(facet_id);
            }

            let exec =
                match raw_effect.exec {
                    RawEffectExec::Static(argv) => EffectExec::Static(parse_argv(&name, argv)?),
                    RawEffectExec::Facet {
                        facet,
                        mut variants,
                    } => {
                        let facet_id = facets.get_index_of(facet.as_ref()).map(FacetId).context(
                            format_args!(
                                "effect '{name}' command references unknown facet '{facet}'"
                            ),
                        )?;
                        if on.as_slice() != [facet_id] {
                            return Err(error!(
                                "effect '{name}' with a facet command must set on = ['{facet}']"
                            ));
                        }

                        let definition = &facets.get_index(facet_id.0).unwrap().1;
                        if variants.len() != definition.variants.len()
                            || variants
                                .keys()
                                .any(|variant| !definition.variants.contains_key(variant.as_ref()))
                        {
                            return Err(error!(
                                "effect '{name}' command variants do not match facet '{facet}'"
                            ));
                        }
                        let mut commands = Vec::with_capacity(variants.len());
                        for variant in definition.variants.keys() {
                            let argv =
                                variants
                                    .shift_remove(variant.as_ref())
                                    .context(format_args!(
                                "effect '{name}' command variants do not match facet '{facet}'"
                            ))?;
                            commands.push(parse_argv(&name, argv)?);
                        }
                        EffectExec::Facet {
                            facet: facet_id,
                            variants: commands.into_boxed_slice(),
                        }
                    }
                };
            effects.push(Effect {
                name,
                on: on.into_boxed_slice(),
                exec,
                ignore_failure: raw_effect.ignore_failure,
            });
        }

        Ok(Self {
            home: raw.home,
            facets,
            files: files.into_boxed_slice(),
            effects: effects.into_boxed_slice(),
        })
    }

    pub fn facet_id(&self, name: &str) -> Option<FacetId> {
        self.facets.get_index_of(name).map(FacetId)
    }

    pub fn facets(&self) -> impl ExactSizeIterator<Item = (FacetId, &str, &Facet)> {
        self.facets
            .iter()
            .enumerate()
            .map(|(index, (name, facet))| (FacetId(index), name.as_ref(), facet))
    }

    pub fn default_selection(&self) -> Selection {
        Selection(
            self.facets
                .values()
                .map(|facet| facet.default)
                .collect::<Vec<_>>()
                .into_boxed_slice(),
        )
    }

    pub fn parse_selection(&self, raw: RawSelection) -> crate::Result<Selection> {
        if raw.len() != self.facets.len() {
            return Err(error!("selection does not contain every manifest facet"));
        }

        let mut selection = self.default_selection();
        for (name, value) in raw {
            let facet_id = self
                .facet_id(&name)
                .context(format_args!("selection contains unknown facet '{name}'"))?;
            let facet = &self[facet_id];
            let variant_id = facet.variant_id(&value).context(format_args!(
                "selection contains invalid value '{value}' for facet '{name}'"
            ))?;
            selection[facet_id] = variant_id;
        }

        Ok(selection)
    }
}

impl Facet {
    pub fn variant_id(&self, name: &str) -> Option<VariantId> {
        self.variants.get_index_of(name).map(VariantId)
    }

    pub fn variant(&self, id: VariantId) -> (&str, &Variant) {
        let (name, variant) = self.variants.get_index(id.0).unwrap();
        (name, variant)
    }
}

impl EffectExec {
    pub fn resolve<'a>(&'a self, selection: &Selection) -> &'a Argv {
        match self {
            Self::Static(argv) => argv,
            Self::Facet { facet, variants } => &variants[selection[*facet].0],
        }
    }
}

impl Index<FacetId> for Config {
    type Output = Facet;

    fn index(&self, index: FacetId) -> &Self::Output {
        self.facets.get_index(index.0).unwrap().1
    }
}

impl Index<VariantId> for Facet {
    type Output = Variant;

    fn index(&self, index: VariantId) -> &Self::Output {
        self.variants.get_index(index.0).unwrap().1
    }
}

impl Index<FacetId> for Selection {
    type Output = VariantId;

    fn index(&self, index: FacetId) -> &Self::Output {
        &self.0[index.0]
    }
}

impl IndexMut<FacetId> for Selection {
    fn index_mut(&mut self, index: FacetId) -> &mut Self::Output {
        &mut self.0[index.0]
    }
}

pub struct NamedSelection<'a> {
    pub config: &'a Config,
    pub selection: &'a Selection,
}

impl Serialize for NamedSelection<'_> {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut map = serializer.serialize_map(Some(self.config.facets.len()))?;
        for (index, (name, facet)) in self.config.facets.iter().enumerate() {
            let variant_id = self.selection[FacetId(index)];
            let variant_name = facet.variants.get_index(variant_id.0).unwrap().0;
            map.serialize_entry(name.as_ref(), variant_name.as_ref())?;
        }
        map.end()
    }
}

fn parse_argv(name: &str, argv: Vec<Box<str>>) -> crate::Result<Argv> {
    let Some(executable) = argv.first() else {
        return Err(error!("effect '{name}' command is empty"));
    };
    if !Path::new(executable.as_ref()).is_absolute() {
        return Err(error!("effect '{name}' executable must be absolute"));
    }
    Ok(argv.into_boxed_slice())
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawManifest {
    version: u64,
    home: PathBuf,
    facets: IndexMap<Box<str>, RawFacet>,
    #[serde(default)]
    files: IndexMap<Box<str>, RawManagedFile>,
    #[serde(default)]
    effects: IndexMap<Box<str>, RawEffect>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawFacet {
    default: Box<str>,
    variants: IndexMap<Box<str>, PathBuf>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged, deny_unknown_fields)]
enum RawManagedFile {
    Static(PathBuf),
    Facet { facet: Box<str> },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawEffect {
    #[serde(default)]
    on: Vec<Box<str>>,
    exec: RawEffectExec,
    #[serde(default, rename = "ignoreFailure")]
    ignore_failure: bool,
}

#[derive(Debug, Deserialize)]
#[serde(untagged, deny_unknown_fields)]
enum RawEffectExec {
    Static(Vec<Box<str>>),
    Facet {
        facet: Box<str>,
        variants: IndexMap<Box<str>, Vec<Box<str>>>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const VALID_CONFIG: &str = r#"{
        "version": 4,
        "home": "/home/test",
        "facets": {
            "theme": {
                "default": "dark",
                "variants": {
                    "light": "/nix/store/light",
                    "dark": "/nix/store/dark"
                }
            }
        },
        "files": {
            ".config/app": {"facet": "theme"},
            ".config/static": "/nix/store/static"
        },
        "effects": {
            "always": {"exec": ["/bin/true"], "ignoreFailure": true},
            "reload": {
                "on": ["theme"],
                "exec": {
                    "facet": "theme",
                    "variants": {
                        "dark": ["/bin/true", "dark"],
                        "light": ["/bin/true", "light"]
                    }
                }
            }
        }
    }"#;

    fn parse(value: serde_json::Value) -> crate::Result<Config> {
        let encoded = serde_json::to_vec(&value).unwrap();
        Config::parse(encoded.as_slice())
    }

    fn valid_config() -> serde_json::Value {
        serde_json::from_str(VALID_CONFIG).unwrap()
    }

    #[test]
    fn resolves_manifest_and_selection() {
        let config = Config::parse(VALID_CONFIG.as_bytes()).unwrap();
        let theme = config.facet_id("theme").unwrap();
        let facet = &config[theme];

        assert_eq!(theme, FacetId(0));
        assert_eq!(facet.default, VariantId(1));
        assert!(matches!(config.files[0].source, Source::Facet(id) if id == theme));
        assert!(config.effects[0].ignore_failure);
        assert_eq!(config.effects[1].on.as_ref(), [theme]);
        assert!(matches!(
            &config.effects[1].exec,
            EffectExec::Facet { facet, variants }
                if *facet == theme && variants[VariantId(1).0][1].as_ref() == "dark"
        ));
        let raw: RawSelection = serde_json::from_str(r#"{"theme":"light"}"#).unwrap();
        let selection = config.parse_selection(raw).unwrap();

        assert_eq!(selection[theme], VariantId(0));
        assert_eq!(
            serde_json::to_value(NamedSelection {
                config: &config,
                selection: &selection,
            })
            .unwrap(),
            json!({"theme": "light"})
        );
    }

    #[test]
    fn rejects_invalid_manifest() {
        let mut value = valid_config();
        value["facets"]["theme"]["default"] = json!("missing");
        assert!(parse(value).is_err());

        let mut value = valid_config();
        value["files"][".config/app"]["facet"] = json!("missing");
        assert!(parse(value).is_err());

        let mut value = valid_config();
        value["effects"]["reload"]["exec"]["variants"]
            .as_object_mut()
            .unwrap()
            .remove("dark");
        assert!(parse(value).is_err());

        for path in ["../outside", "/absolute", "a//b", "a/./b"] {
            let mut value = valid_config();
            let files = value["files"].as_object_mut().unwrap();
            files.clear();
            files.insert(path.to_string(), json!({"facet": "theme"}));
            assert!(parse(value).is_err());
        }
    }
}
