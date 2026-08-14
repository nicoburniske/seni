{
  config,
  lib,
  pkgs,
  ...
}: let
  types = lib.types;
  cfg = config.sumi;

  fileKinds = ["home" "config" "cache" "data" "state"];

  valueType = types.oneOf [types.str types.path];
  argvType = types.listOf valueType;

  facetOptionType = types.submodule {
    options = {
      default = lib.mkOption {
        type = types.str;
        description = "default variant";
      };

      variants = lib.mkOption {
        type = types.attrsOf types.anything;
        default = {};
        description = "variant payloads";
      };
    };
  };

  fileOptionType = types.submodule {
    options = {
      facet = lib.mkOption {
        type = types.nullOr types.str;
        default = null;
        description = "facet used to generate this file";
      };

      value = lib.mkOption {
        type = types.oneOf [valueType (types.functionTo valueType)];
        description = "file contents or source";
      };
    };
  };

  effectOptionType = types.submodule {
    options = {
      on = lib.mkOption {
        type = types.listOf types.str;
        default = [];
        description = "facets that trigger this effect during a switch";
      };

      exec = lib.mkOption {
        type = types.oneOf [argvType (types.functionTo argvType)];
        description = "effect argv";
      };

      ignoreFailure = lib.mkOption {
        type = types.bool;
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
        name = lib.strings.sanitizeDerivationName "sumi-${baseNameOf (toString value)}";
        path = value;
      };
    }
    else if builtins.isList value
    then map materialize value
    else if builtins.isAttrs value
    then lib.mapAttrs (_: materialize) value
    else value;

  renderArgv = map (argument: toString (materialize argument));
in {
  options.sumi = {
    enable = lib.mkEnableOption "Sumi facet-based runtime config switching";

    facet = lib.mkOption {
      type = types.attrsOf facetOptionType;
      default = {};
      description = "facets keyed by name";
    };

    file = lib.genAttrs fileKinds (kind:
      lib.mkOption {
        type = types.attrsOf fileOptionType;
        default = {};
        description = "files in the ${kind} directory";
      });

    effect = lib.mkOption {
      type = types.attrsOf effectOptionType;
      default = {};
      description = "effects keyed by name";
    };

    path = {
      home = lib.mkOption {
        type = types.str;
        description = "home directory managed by Sumi";
      };

      config = lib.mkOption {
        type = types.str;
        default = "${cfg.path.home}/.config";
        description = "XDG config directory";
      };

      cache = lib.mkOption {
        type = types.str;
        default = "${cfg.path.home}/.cache";
        description = "XDG cache directory";
      };

      data = lib.mkOption {
        type = types.str;
        default = "${cfg.path.home}/.local/share";
        description = "XDG data directory";
      };

      state = lib.mkOption {
        type = types.str;
        default = "${cfg.path.home}/.local/state";
        description = "XDG state directory";
      };
    };

    stateDirectory = lib.mkOption {
      type = types.str;
      default = "${cfg.path.state}/sumi";
      description = "Sumi runtime state directory";
    };

    generated.manifest = lib.mkOption {
      type = types.path;
      readOnly = true;
      internal = true;
    };

    package = lib.mkOption {
      type = types.package;
      readOnly = true;
      internal = true;
    };
  };

  config = lib.mkIf cfg.enable (let
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
          pkgs.runCommandLocal (lib.strings.sanitizeDerivationName "sumi-${facet}-${variant}") {} ''
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
            then toString (pkgs.writeText (lib.strings.sanitizeDerivationName "sumi-${file.path}") file.value)
            else toString (materialize file.value);
        })
        files);
      effects = manifestEffects;
    };
    manifestPath = pkgs.writeText "sumi-manifest.json" (builtins.toJSON manifest);
    wrapperEnvironment = {
      XDG_CONFIG_HOME = cfg.path.config;
      XDG_CACHE_HOME = cfg.path.cache;
      XDG_DATA_HOME = cfg.path.data;
      XDG_STATE_HOME = cfg.path.state;
      SUMI_MANIFEST = manifestPath;
      SUMI_STATE_DIR = cfg.stateDirectory;
    };
    wrappedCli = pkgs.symlinkJoin {
      name = "sumi";
      paths = [(pkgs.callPackage ./cli/default.nix {})];
      nativeBuildInputs = [pkgs.makeWrapper];
      postBuild = ''
        wrapProgram "$out/bin/sumi" ${lib.escapeShellArgs (lib.concatMap (name: ["--set" name (toString wrapperEnvironment.${name})]) (builtins.attrNames wrapperEnvironment))}
      '';
    };
  in {
    assertions = [
      {
        assertion = lib.all (directory:
          lib.hasPrefix "/" directory
          && directory != "/"
          && lib.all (segment: segment != "" && segment != "." && segment != "..") (lib.drop 1 (lib.splitString "/" directory)))
        ((builtins.attrValues directories) ++ [cfg.stateDirectory]);
        message = "Sumi directories must be normalized absolute paths";
      }
      {
        assertion = cfg.facet != {};
        message = "sumi.facet must define at least one facet";
      }
      {
        assertion = lib.all (name: name != "" && name != "." && name != ".." && !lib.hasInfix "/" name) (builtins.attrNames cfg.facet);
        message = "Sumi facet names must be single path segments";
      }
      {
        assertion = lib.all (facet: facet.variants != {} && builtins.hasAttr facet.default facet.variants && lib.all (variant: variant != "") (builtins.attrNames facet.variants)) (builtins.attrValues cfg.facet);
        message = "Sumi facets must have variants and a valid default";
      }
      {
        assertion = lib.all (kind: cfg.file.${kind} == {} || roots.${kind} != null) fileKinds;
        message = "Sumi file directories must be within sumi.path.home";
      }
      {
        assertion = lib.all (file: file.valid) files;
        message = "Sumi file paths must be normalized relative paths";
      }
      {
        assertion = lib.all (file: lib.isFunction file.value == (file.facet != null)) files;
        message = "Sumi function file values require a facet, and static values cannot set one";
      }
      {
        assertion = lib.all (file: file.facet == null || builtins.hasAttr file.facet cfg.facet) files;
        message = "Sumi files must reference defined facets";
      }
      {
        assertion = lib.all (matches: lib.length matches == 1) (builtins.attrValues filesByPath);
        message = "Sumi file destinations must be unique";
      }
      {
        assertion =
          !lib.any (path: let
            segments = lib.splitString "/" path;
          in
            lib.any (count: builtins.hasAttr (lib.concatStringsSep "/" (lib.take count segments)) filesByPath) (lib.range 1 (lib.length segments - 1)))
          managedPaths;
        message = "Sumi file destinations cannot contain one another";
      }
      {
        assertion = lib.all (name: let
          effect = cfg.effect.${name};
        in
          name
          != ""
          && lib.all (facet: builtins.hasAttr facet cfg.facet) effect.on
          && (!lib.isFunction effect.exec || lib.length effect.on == 1))
        (builtins.attrNames cfg.effect);
        message = "Sumi effects must reference defined facets, and function effects require exactly one facet";
      }
      {
        assertion = lib.all (effect:
          if builtins.isList effect.exec
          then effect.exec != [] && lib.hasPrefix "/" (builtins.head effect.exec)
          else lib.all (argv: argv != [] && lib.hasPrefix "/" (builtins.head argv)) (builtins.attrValues effect.exec.variants))
        (builtins.attrValues manifestEffects);
        message = "Sumi effect commands must have an absolute executable";
      }
    ];

    environment.systemPackages = [wrappedCli];

    sumi = {
      generated.manifest = manifestPath;
      package = wrappedCli;
    };
  });
}
