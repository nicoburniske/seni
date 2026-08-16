{existingFileStrategy}: {
  config,
  lib,
  name,
  pkgs,
  ...
}: let
  cfg = config;
  inherit (lib) mkOption types;

  fileKinds = ["home" "config" "cache" "data" "state"];
  home = cfg.path.home;

  valueType = types.oneOf [types.str types.path];
  argvType = types.listOf valueType;

  validSegment = segment: segment != "" && segment != "." && segment != "..";
  validCommand = argv: argv != [] && lib.hasPrefix "/" (builtins.head argv);

  facetOptionType = types.submodule {
    options = {
      default = mkOption {
        type = types.str;
        description = "default variant";
      };

      variants = mkOption {
        type = types.attrsOf types.anything;
        default = {};
        description = "variant payloads";
      };
    };
  };

  effectOptions = {
    exec = mkOption {
      type = types.oneOf [argvType (types.functionTo argvType)];
      description = "effect argv";
    };

    ignoreFailure = mkOption {
      type = types.bool;
      default = false;
      description = "whether to ignore a nonzero exit status";
    };
  };

  fileEffectOptionType = types.submodule {options = effectOptions;};

  fileOptionType = types.submodule {
    options = {
      facet = mkOption {
        type = types.nullOr (types.coercedTo types.str lib.singleton (types.listOf types.str));
        default = null;
        description = "facets used to generate this file";
      };

      value = mkOption {
        type = types.oneOf [valueType (types.functionTo valueType)];
        description = "file contents or source";
      };

      effect = mkOption {
        type = types.nullOr fileEffectOptionType;
        default = null;
        description = "effect triggered by this file's facet, or every switch for a static file";
      };
    };
  };

  effectOptionType = types.submodule {
    options = effectOptions // {
      on = mkOption {
        type = types.listOf types.str;
        default = [];
        description = "facets that trigger this effect during a switch";
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
  directoryRoots = lib.mapAttrs (_: directory:
    if directory == home
    then ""
    else if lib.hasPrefix "${home}/" directory
    then lib.removePrefix "${home}/" directory
    else null)
  directories;

  files = lib.concatMap (kind:
    lib.mapAttrsToList (path: file: let
      facets =
        if file.facet == null
        then []
        else lib.sort builtins.lessThan file.facet;
    in {
      path =
        if directoryRoots.${kind} == null || directoryRoots.${kind} == ""
        then path
        else "${directoryRoots.${kind}}/${path}";
      valid = lib.all validSegment (lib.splitString "/" path);
      rootKey = builtins.toJSON facets;
      inherit facets;
      inherit (file) effect value;
    })
    cfg.file.${kind})
  fileKinds;
  filesByPath = lib.groupBy (file: file.path) files;
  managedPaths = builtins.attrNames filesByPath;
  dynamicFilesByRoot = lib.pipe files [
    (lib.filter (file: lib.isFunction file.value))
    (lib.groupBy (file: file.rootKey))
  ];

  resolvedFacets = materialize cfg.facet;
  context = facet: variant: {
    inherit variant;
    value = resolvedFacets.${facet}.variants.${variant};
  };

  buildRoot = rootName: dynamicFiles: fileContext:
    pkgs.runCommandLocal (lib.strings.sanitizeDerivationName rootName) {} ''
      set -eu
      mkdir -p "$out"
      ${lib.concatMapStringsSep "\n" (file: let
        value = materialize (file.value fileContext);
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
    '';

  roots = lib.mapAttrsToList (key: dynamicFiles: let
    facets = (builtins.head dynamicFiles).facets;
    selections = lib.cartesianProduct (lib.genAttrs facets (facet: builtins.attrNames cfg.facet.${facet}.variants));
  in {
    inherit facets key;
    variants = map (selection:
      buildRoot
      ("seni-${name}-" + lib.concatMapStringsSep "-" (facet: "${facet}-${selection.${facet}}") facets)
      dynamicFiles
      (lib.mapAttrs context selection))
    selections;
  })
  dynamicFilesByRoot;
  rootIndices = lib.pipe roots [
    (lib.imap0 (index: root: lib.nameValuePair root.key index))
    builtins.listToAttrs
  ];

  fileEffects = lib.pipe files [
    (lib.filter (file: file.effect != null))
    (map (file: {
      name = "file:${file.path}";
      value =
        file.effect
        // {
          on = file.facets;
        };
    }))
    builtins.listToAttrs
  ];
  effects = cfg.effect // fileEffects;

  manifestEffects =
    lib.mapAttrs (_: effect: let
      exec = effect.exec;
    in {
      inherit (effect) on ignoreFailure;
      exec =
        if lib.isFunction exec
        then let
          facet =
            if effect.on == []
            then ""
            else builtins.head effect.on;
        in {
          inherit facet;
          variants =
            if builtins.hasAttr facet cfg.facet
            then lib.mapAttrs (variant: _: renderArgv (exec {${facet} = context facet variant;})) cfg.facet.${facet}.variants
            else {};
        }
        else renderArgv exec;
    })
    effects;

  manifest = {
    version = 1;
    inherit existingFileStrategy home;
    facets =
      lib.mapAttrs (facet: data: {
        inherit (data) default;
        variants = builtins.attrNames data.variants;
      })
      cfg.facet;
    roots = map (root: {
      inherit (root) facets;
      variants = map toString root.variants;
    }) roots;
    files = lib.pipe files [
      (map (file: {
        name = file.path;
        value =
          if lib.isFunction file.value
          then {root = rootIndices.${file.rootKey};}
          else if builtins.isString file.value
          then toString (pkgs.writeText (lib.strings.sanitizeDerivationName "seni-${name}-${file.path}") file.value)
          else toString (materialize file.value);
      }))
      builtins.listToAttrs
    ];
    effects = manifestEffects;
  };
in {
  _class = "seni";

  imports = [(pkgs.path + "/nixos/modules/misc/assertions.nix")];

  options = {
    enable = mkOption {
      type = types.bool;
      default = true;
      description = "whether to manage this user's Seni configuration";
    };

    facet = mkOption {
      type = types.attrsOf facetOptionType;
      default = {};
      description = "facets keyed by name";
    };

    file = lib.genAttrs fileKinds (kind:
      mkOption {
        type = types.attrsOf fileOptionType;
        default = {};
        description = "files in the ${kind} directory";
      });

    effect = mkOption {
      type = types.attrsOf effectOptionType;
      default = {};
      description = "effects keyed by name";
    };

    packages = mkOption {
      type = types.listOf types.package;
      default = [];
      description = "packages installed for this user";
    };

    environment = {
      sessionVariables = mkOption {
        type = types.attrsOf types.str;
        default = {};
        description = "environment variables for this user";
      };

      loadEnv = mkOption {
        type = types.path;
        readOnly = true;
        description = "script that exports this user's environment variables";
      };
    };

    path = {
      home = mkOption {
        type = types.str;
        readOnly = true;
        description = "home directory managed by Seni";
      };

      config = mkOption {
        type = types.str;
        default = "${home}/.config";
        description = "XDG config directory";
      };

      cache = mkOption {
        type = types.str;
        default = "${home}/.cache";
        description = "XDG cache directory";
      };

      data = mkOption {
        type = types.str;
        default = "${home}/.local/share";
        description = "XDG data directory";
      };

      state = mkOption {
        type = types.str;
        default = "${home}/.local/state";
        description = "XDG state directory";
      };
    };

    generated.manifest = mkOption {
      type = types.path;
      readOnly = true;
      internal = true;
    };
  };

  config = {
    environment = {
      sessionVariables = {
        XDG_CACHE_HOME = cfg.path.cache;
        XDG_CONFIG_HOME = cfg.path.config;
        XDG_DATA_HOME = cfg.path.data;
        XDG_STATE_HOME = cfg.path.state;
      };
      loadEnv = lib.pipe cfg.environment.sessionVariables [
        (lib.mapAttrsToList (variable: value: "export ${variable}=${lib.escapeShellArg value}"))
        (lib.concatStringsSep "\n")
        (pkgs.writeShellScript "seni-${name}-environment")
      ];
    };

    assertions = lib.mkIf cfg.enable [
      {
        assertion = lib.all (directory:
          lib.hasPrefix "/" directory
          && directory != "/"
          && lib.all validSegment (lib.drop 1 (lib.splitString "/" directory)))
        (builtins.attrValues directories);
        message = "directories must be normalized absolute paths";
      }
      {
        assertion = cfg.facet != {};
        message = "facet must define at least one facet";
      }
      {
        assertion = lib.all (facet: validSegment facet && !lib.hasInfix "/" facet) (builtins.attrNames cfg.facet);
        message = "facet names must be single path segments";
      }
      {
        assertion = lib.all (facet: facet.variants != {} && builtins.hasAttr facet.default facet.variants && lib.all (variant: variant != "") (builtins.attrNames facet.variants)) (builtins.attrValues cfg.facet);
        message = "facets must have variants and a valid default";
      }
      {
        assertion = lib.all (kind: cfg.file.${kind} == {} || directoryRoots.${kind} != null) fileKinds;
        message = "file directories must be within the home directory";
      }
      {
        assertion = lib.all (file: file.valid) files;
        message = "file paths must be normalized relative paths";
      }
      {
        assertion = lib.all (file: lib.isFunction file.value == (file.facets != [])) files;
        message = "function file values require a facet, and static values cannot set one";
      }
      {
        assertion = lib.all (file: lib.allUnique file.facets && lib.all (facet: builtins.hasAttr facet cfg.facet) file.facets) files;
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
        assertion = let
          state = "${cfg.path.state}/seni";
          relative = lib.removePrefix "${home}/" state;
        in
          !lib.hasPrefix "${home}/" state
          || !lib.any (path:
            path == relative
            || lib.hasPrefix "${relative}/" path
            || lib.hasPrefix "${path}/" relative)
          managedPaths;
        message = "file destinations cannot overlap the Seni state directory";
      }
      {
        assertion = lib.all (effectName: let
          effect = effects.${effectName};
        in
          effectName
          != ""
          && lib.all (facet: builtins.hasAttr facet cfg.facet) effect.on
          && (!lib.isFunction effect.exec || lib.length effect.on == 1))
        (builtins.attrNames effects);
        message = "effects must reference defined facets, and function effects require exactly one facet";
      }
      {
        assertion = lib.intersectLists (builtins.attrNames cfg.effect) (builtins.attrNames fileEffects) == [];
        message = "file effects must not conflict with explicitly named effects";
      }
      {
        assertion = lib.all (effect: let
          exec = effect.exec;
        in
          if builtins.isList exec
          then validCommand exec
          else lib.all validCommand (builtins.attrValues exec.variants))
        (builtins.attrValues manifestEffects);
        message = "effect commands must have an absolute executable";
      }
    ];

    generated.manifest = lib.mkIf cfg.enable (pkgs.writeText "seni-${name}-manifest.json" (builtins.toJSON manifest));
  };
}
