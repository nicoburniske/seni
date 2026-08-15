{
  config,
  lib,
  name,
  pkgs,
  ...
}: let
  cfg = config;
  fileKinds = ["home" "config" "cache" "data" "state"];

  valueType = lib.types.oneOf [lib.types.str lib.types.path];
  argvType = lib.types.listOf valueType;

  facetOptionType = lib.types.submodule {
    options = {
      default = lib.mkOption {
        type = lib.types.str;
        description = "default variant";
      };

      variants = lib.mkOption {
        type = lib.types.attrsOf lib.types.anything;
        default = {};
        description = "variant payloads";
      };
    };
  };

  fileOptionType = lib.types.submodule {
    options = {
      facet = lib.mkOption {
        type = lib.types.nullOr lib.types.str;
        default = null;
        description = "facet used to generate this file";
      };

      value = lib.mkOption {
        type = lib.types.oneOf [valueType (lib.types.functionTo valueType)];
        description = "file contents or source";
      };
    };
  };

  effectOptionType = lib.types.submodule {
    options = {
      on = lib.mkOption {
        type = lib.types.listOf lib.types.str;
        default = [];
        description = "facets that trigger this effect during a switch";
      };

      exec = lib.mkOption {
        type = lib.types.oneOf [argvType (lib.types.functionTo argvType)];
        description = "effect argv";
      };

      ignoreFailure = lib.mkOption {
        type = lib.types.bool;
        default = false;
        description = "whether to ignore a nonzero exit status";
      };
    };
  };

  materialize = value:
    if lib.isDerivation value
    then value
    else if builtins.isPath value
    then {
      outPath = builtins.path {
        name = lib.strings.sanitizeDerivationName "seni-${baseNameOf (toString value)}";
        path = value;
      };
    }
    else if builtins.isList value
    then map materialize value
    else if builtins.isAttrs value
    then lib.mapAttrs (_: materialize) value
    else value;

  renderArgv = map (argument: toString (materialize argument));

  directories = cfg.path;
  roots = lib.mapAttrs (_: directory:
    if directory == cfg.path.home
    then ""
    else if lib.hasPrefix "${cfg.path.home}/" directory
    then lib.removePrefix "${cfg.path.home}/" directory
    else null)
  directories;

  files = lib.concatMap (kind:
    lib.mapAttrsToList (path: file: {
      path =
        if roots.${kind} == null || roots.${kind} == ""
        then path
        else "${roots.${kind}}/${path}";
      valid = lib.all (segment: segment != "" && segment != "." && segment != "..") (lib.splitString "/" path);
      inherit (file) facet value;
    })
    cfg.file.${kind})
  fileKinds;
  filesByPath = lib.groupBy (file: file.path) files;
  managedPaths = builtins.attrNames filesByPath;
  dynamicFilesByFacet = lib.groupBy (file: file.facet) (lib.filter (file: lib.isFunction file.value && file.facet != null) files);

  resolvedFacets = materialize cfg.facet;
  context = facet: variant: {
    inherit variant;
    value = resolvedFacets.${facet}.variants.${variant};
  };

  variantRoots = lib.mapAttrs (facet: data: let
    dynamicFiles = dynamicFilesByFacet.${facet} or [];
  in
    lib.mapAttrs (variant: _:
      if dynamicFiles == []
      then pkgs.emptyDirectory
      else
        pkgs.runCommandLocal (lib.strings.sanitizeDerivationName "seni-${name}-${facet}-${variant}") {} ''
          set -eu
          mkdir -p "$out"
          ${lib.concatMapStringsSep "\n" (file: let
            value = materialize (file.value (context facet variant));
          in
            ''
              target="$out"/${lib.escapeShellArg file.path}
              mkdir -p "$(dirname "$target")"
            ''
            + (
              if builtins.isString value
              then ''printf '%s' ${lib.escapeShellArg value} > "$target"''
              else ''ln -s ${lib.escapeShellArg (toString value)} "$target"''
            ))
          dynamicFiles}
        '')
    data.variants)
  cfg.facet;

  manifestEffects =
    lib.mapAttrs (_: effect: {
      inherit (effect) on ignoreFailure;
      exec =
        if lib.isFunction effect.exec
        then let
          facet =
            if effect.on == []
            then ""
            else builtins.head effect.on;
        in {
          inherit facet;
          variants =
            if builtins.hasAttr facet cfg.facet
            then lib.mapAttrs (variant: _: renderArgv (effect.exec (context facet variant))) cfg.facet.${facet}.variants
            else {};
        }
        else renderArgv effect.exec;
    })
    cfg.effect;

  manifest = {
    version = 4;
    home = cfg.path.home;
    facets =
      lib.mapAttrs (facet: data: {
        inherit (data) default;
        variants = lib.mapAttrs (variant: _: toString variantRoots.${facet}.${variant}) data.variants;
      })
      cfg.facet;
    files = builtins.listToAttrs (map (file: {
        name = file.path;
        value =
          if lib.isFunction file.value
          then {facet = file.facet;}
          else if builtins.isString file.value
          then toString (pkgs.writeText (lib.strings.sanitizeDerivationName "seni-${name}-${file.path}") file.value)
          else toString (materialize file.value);
      })
      files);
    effects = manifestEffects;
  };
in {
  _class = "seni";

  imports = [(pkgs.path + "/nixos/modules/misc/assertions.nix")];

  options = {
    enable = lib.mkOption {
      type = lib.types.bool;
      default = true;
      description = "whether to manage this user's Seni configuration";
    };

    facet = lib.mkOption {
      type = lib.types.attrsOf facetOptionType;
      default = {};
      description = "facets keyed by name";
    };

    file = lib.genAttrs fileKinds (kind:
      lib.mkOption {
        type = lib.types.attrsOf fileOptionType;
        default = {};
        description = "files in the ${kind} directory";
      });

    effect = lib.mkOption {
      type = lib.types.attrsOf effectOptionType;
      default = {};
      description = "effects keyed by name";
    };

    packages = lib.mkOption {
      type = lib.types.listOf lib.types.package;
      default = [];
      description = "packages installed for this user";
    };

    path = {
      home = lib.mkOption {
        type = lib.types.str;
        readOnly = true;
        description = "home directory managed by Seni";
      };

      config = lib.mkOption {
        type = lib.types.str;
        default = "${cfg.path.home}/.config";
        description = "XDG config directory";
      };

      cache = lib.mkOption {
        type = lib.types.str;
        default = "${cfg.path.home}/.cache";
        description = "XDG cache directory";
      };

      data = lib.mkOption {
        type = lib.types.str;
        default = "${cfg.path.home}/.local/share";
        description = "XDG data directory";
      };

      state = lib.mkOption {
        type = lib.types.str;
        default = "${cfg.path.home}/.local/state";
        description = "XDG state directory";
      };
    };

    generated.manifest = lib.mkOption {
      type = lib.types.path;
      readOnly = true;
      internal = true;
    };
  };

  config = lib.mkIf cfg.enable {
    assertions = [
      {
        assertion = lib.all (directory:
          lib.hasPrefix "/" directory
          && directory != "/"
          && lib.all (segment: segment != "" && segment != "." && segment != "..") (lib.drop 1 (lib.splitString "/" directory)))
        (builtins.attrValues directories);
        message = "directories must be normalized absolute paths";
      }
      {
        assertion = cfg.facet != {};
        message = "facet must define at least one facet";
      }
      {
        assertion = lib.all (facet: facet != "" && facet != "." && facet != ".." && !lib.hasInfix "/" facet) (builtins.attrNames cfg.facet);
        message = "facet names must be single path segments";
      }
      {
        assertion = lib.all (facet: facet.variants != {} && builtins.hasAttr facet.default facet.variants && lib.all (variant: variant != "") (builtins.attrNames facet.variants)) (builtins.attrValues cfg.facet);
        message = "facets must have variants and a valid default";
      }
      {
        assertion = lib.all (kind: cfg.file.${kind} == {} || roots.${kind} != null) fileKinds;
        message = "file directories must be within the home directory";
      }
      {
        assertion = lib.all (file: file.valid) files;
        message = "file paths must be normalized relative paths";
      }
      {
        assertion = lib.all (file: lib.isFunction file.value == (file.facet != null)) files;
        message = "function file values require a facet, and static values cannot set one";
      }
      {
        assertion = lib.all (file: file.facet == null || builtins.hasAttr file.facet cfg.facet) files;
        message = "files must reference defined facets";
      }
      {
        assertion = lib.all (matches: lib.length matches == 1) (builtins.attrValues filesByPath);
        message = "file destinations must be unique";
      }
      {
        assertion =
          !lib.any (path: let
            segments = lib.splitString "/" path;
          in
            lib.any (count: builtins.hasAttr (lib.concatStringsSep "/" (lib.take count segments)) filesByPath) (lib.range 1 (lib.length segments - 1)))
          managedPaths;
        message = "file destinations cannot contain one another";
      }
      {
        assertion = lib.all (effectName: let
          effect = cfg.effect.${effectName};
        in
          effectName
          != ""
          && lib.all (facet: builtins.hasAttr facet cfg.facet) effect.on
          && (!lib.isFunction effect.exec || lib.length effect.on == 1))
        (builtins.attrNames cfg.effect);
        message = "effects must reference defined facets, and function effects require exactly one facet";
      }
      {
        assertion = lib.all (effect:
          if builtins.isList effect.exec
          then effect.exec != [] && lib.hasPrefix "/" (builtins.head effect.exec)
          else lib.all (argv: argv != [] && lib.hasPrefix "/" (builtins.head argv)) (builtins.attrValues effect.exec.variants))
        (builtins.attrValues manifestEffects);
        message = "effect commands must have an absolute executable";
      }
    ];

    generated.manifest = pkgs.writeText "seni-${name}-manifest.json" (builtins.toJSON manifest);
  };
}
