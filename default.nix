{
  config,
  lib,
  pkgs,
  ...
}: let
  types = lib.types;
  cfg = config.sumi;

  sourceType = with types; oneOf [str path package];
  argvType = types.listOf sourceType;

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
        type = with types; nullOr str;
        default = null;
        description = "facet used to generate this file";
      };

      value = lib.mkOption {
        type = with types; nullOr (oneOf [sourceType (functionTo sourceType)]);
        default = null;
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
        type = with types; nullOr (oneOf [argvType (functionTo argvType)]);
        default = null;
        description = "effect argv";
      };
    };
  };

  stripLeadingDotSlash = path:
    if lib.hasPrefix "./" path
    then
      stripLeadingDotSlash
      (builtins.substring 2 ((builtins.stringLength path) - 2) path)
    else path;

  validateRelativePath = path: let
    normalized = stripLeadingDotSlash path;
    segments = lib.splitString "/" normalized;
  in
    lib.filter (error: error != null) [
      (
        if normalized == ""
        then "path must not be empty"
        else null
      )
      (
        if lib.hasPrefix "/" path
        then "path must be relative"
        else null
      )
      (
        if builtins.elem "." segments
        then "path must not contain '.'"
        else null
      )
      (
        if builtins.elem ".." segments
        then "path must not contain '..'"
        else null
      )
      (
        if builtins.elem "" segments
        then "path must not contain empty segments"
        else null
      )
    ];

  toHomeRelative = home: absolute:
    if absolute == home
    then ""
    else if lib.hasPrefix "${home}/" absolute
    then
      builtins.substring
      ((builtins.stringLength home) + 1)
      ((builtins.stringLength absolute) - (builtins.stringLength home) - 1)
      absolute
    else null;

  joinRelative = root: path:
    if root == ""
    then path
    else "${root}/${path}";

  sanitizePath = path:
    lib.replaceStrings
    ["/" "." " " ":" "@" "+" "\\"]
    ["-" "-" "-" "-" "-" "-" "-"]
    path;

  freezePath = path: let
    string = toString path;
  in
    builtins.path {
      name = lib.strings.sanitizeDerivationName "sumi-${baseNameOf string}";
      inherit path;
    };

  materialize = value:
    if lib.isDerivation value
    then value
    else if builtins.isPath value
    then freezePath value
    else if builtins.isList value
    then map materialize value
    else if builtins.isAttrs value
    then lib.mapAttrs (_: materialize) value
    else value;
in {
  options.sumi = {
    enable = lib.mkEnableOption "Sumi facet-based runtime config switching";

    facet = lib.mkOption {
      type = types.attrsOf facetOptionType;
      default = {};
      description = "facets keyed by name";
    };

    file = {
      home = lib.mkOption {
        type = types.attrsOf fileOptionType;
        default = {};
        description = "files relative to the home directory";
      };

      config = lib.mkOption {
        type = types.attrsOf fileOptionType;
        default = {};
        description = "files relative to the XDG config directory";
      };

      cache = lib.mkOption {
        type = types.attrsOf fileOptionType;
        default = {};
        description = "files relative to the XDG cache directory";
      };

      data = lib.mkOption {
        type = types.attrsOf fileOptionType;
        default = {};
        description = "files relative to the XDG data directory";
      };

      state = lib.mkOption {
        type = types.attrsOf fileOptionType;
        default = {};
        description = "files relative to the XDG state directory";
      };
    };

    effect = lib.mkOption {
      type = types.attrsOf effectOptionType;
      default = {};
      description = "effects keyed by name";
    };

    homeDirectory = lib.mkOption {
      type = with types; nullOr str;
      default = null;
      description = "home directory managed by Sumi";
    };

    configHome = lib.mkOption {
      type = with types; nullOr str;
      default = null;
      description = "XDG config directory";
    };

    cacheHome = lib.mkOption {
      type = with types; nullOr str;
      default = null;
      description = "XDG cache directory";
    };

    dataHome = lib.mkOption {
      type = with types; nullOr str;
      default = null;
      description = "XDG data directory";
    };

    stateHome = lib.mkOption {
      type = with types; nullOr str;
      default = null;
      description = "XDG state directory";
    };

    stateDirectory = lib.mkOption {
      type = with types; nullOr str;
      default = null;
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
    home = cfg.homeDirectory;
    homes = {
      home = {
        absolute = home;
        files = cfg.file.home;
      };
      config = {
        absolute =
          if cfg.configHome != null
          then cfg.configHome
          else if home != null
          then "${home}/.config"
          else null;
        files = cfg.file.config;
      };
      cache = {
        absolute =
          if cfg.cacheHome != null
          then cfg.cacheHome
          else if home != null
          then "${home}/.cache"
          else null;
        files = cfg.file.cache;
      };
      data = {
        absolute =
          if cfg.dataHome != null
          then cfg.dataHome
          else if home != null
          then "${home}/.local/share"
          else null;
        files = cfg.file.data;
      };
      state = {
        absolute =
          if cfg.stateHome != null
          then cfg.stateHome
          else if home != null
          then "${home}/.local/state"
          else null;
        files = cfg.file.state;
      };
    };
    stateDirectory =
      if cfg.stateDirectory != null
      then cfg.stateDirectory
      else if homes.state.absolute != null
      then "${homes.state.absolute}/sumi"
      else null;

    facetNames = builtins.attrNames cfg.facet;
    resolvedFacets = materialize cfg.facet;

    homeErrors =
      if home == null
      then []
      else
        lib.filter (error: error != null)
        (lib.mapAttrsToList (name: data:
          if data.absolute != null && toHomeRelative home data.absolute == null
          then "sumi.${name}Home must be within sumi.homeDirectory"
          else null)
        homes);

    facetErrors = lib.flatten (lib.mapAttrsToList (name: facet: let
      variants = builtins.attrNames facet.variants;
    in
      (lib.optional (variants == []) "${name}: variants must not be empty")
      ++ (lib.optional (!(builtins.elem facet.default variants)) "${name}: default '${facet.default}' is not a variant"))
    cfg.facet);

    normalizedFiles = lib.flatten (lib.mapAttrsToList (_: data: let
      root =
        if home != null && data.absolute != null
        then toHomeRelative home data.absolute
        else null;
    in
      if root == null
      then []
      else
        lib.mapAttrsToList (path: file: {
          path = joinRelative root (stripLeadingDotSlash path);
          inherit (file) facet value;
        })
        data.files)
    homes);

    fileErrors = lib.flatten (lib.mapAttrsToList (kind: data:
      lib.flatten (lib.mapAttrsToList (path: file: let
        prefix = "sumi.file.${kind}.${path}";
        pathErrors = map (error: "${prefix}: ${error}") (validateRelativePath path);
        dynamic = lib.isFunction file.value;
      in
        pathErrors
        ++ (lib.optional (file.value == null) "${prefix}: value is required")
        ++ (lib.optional (dynamic && file.facet == null) "${prefix}: function values require facet")
        ++ (lib.optional (!dynamic && file.facet != null) "${prefix}: facet requires a function value")
        ++ (lib.optional (file.facet != null && !(builtins.elem file.facet facetNames)) "${prefix}: unknown facet '${toString file.facet}'"))
      data.files))
    homes);

    filePaths = map (file: file.path) normalizedFiles;
    duplicateFiles =
      lib.filter
      (path: lib.length (lib.filter (candidate: candidate == path) filePaths) > 1)
      (lib.unique filePaths);

    effectErrors = lib.flatten (lib.mapAttrsToList (name: effect:
      (lib.optional (effect.exec == null) "${name}: exec is required")
      ++ (map (facet: "${name}: unknown facet '${facet}'")
        (lib.filter (facet: !(builtins.elem facet facetNames)) effect.on))
      ++ (lib.optional (lib.isFunction effect.exec && lib.length effect.on != 1) "${name}: function exec requires exactly one on facet"))
    cfg.effect);

    context = facet: variant: {
      inherit variant;
      value = resolvedFacets.${facet}.variants.${variant};
    };

    dynamicFiles = facet:
      lib.filter
      (file: lib.isFunction file.value && file.facet == facet)
      normalizedFiles;

    writeVariantFile = facet: variant: file: let
      value = materialize (file.value (context facet variant));
      target = lib.escapeShellArg file.path;
    in
      ''
        target="$out"/${target}
        mkdir -p "$(dirname "$target")"
      ''
      + (
        if builtins.isString value
        then ''printf '%s' ${lib.escapeShellArg value} > "$target"''
        else ''ln -s ${lib.escapeShellArg (toString value)} "$target"''
      );

    variantRoots = lib.mapAttrs (facet: data:
      lib.mapAttrs (variant: _:
        pkgs.runCommandLocal "sumi-${sanitizePath facet}-${sanitizePath variant}" {} ''
          set -eu
          mkdir -p "$out"
          ${lib.concatMapStringsSep "\n" (writeVariantFile facet variant) (dynamicFiles facet)}
        '')
      data.variants)
    cfg.facet;

    manifestFiles = builtins.listToAttrs (map (file: {
        name = file.path;
        value =
          if lib.isFunction file.value
          then {facet = file.facet;}
          else if builtins.isString file.value
          then
            toString (pkgs.writeTextFile {
              name = "sumi-${sanitizePath file.path}";
              text = file.value;
            })
          else toString (materialize file.value);
      })
      normalizedFiles);

    manifestEffects =
      lib.mapAttrs (_: effect: {
        inherit (effect) on;
        exec =
          if lib.isFunction effect.exec
          then let
            facet = builtins.head effect.on;
          in {
            inherit facet;
            variants =
              lib.mapAttrs
              (variant: _: map toString (effect.exec (context facet variant)))
              resolvedFacets.${facet}.variants;
          }
          else map toString effect.exec;
      })
      cfg.effect;

    manifest = {
      version = 4;
      inherit home;
      facets =
        lib.mapAttrs (name: facet: {
          inherit (facet) default;
          variants = lib.mapAttrs (_: toString) variantRoots.${name};
        })
        cfg.facet;
      files = manifestFiles;
      effects = manifestEffects;
    };
    manifestPath = pkgs.writeText "sumi-manifest.json" (builtins.toJSON manifest);
    baseCli = pkgs.callPackage ./cli/default.nix {};

    environment = lib.filterAttrs (_: value: value != null) {
      XDG_CONFIG_HOME = homes.config.absolute;
      XDG_CACHE_HOME = homes.cache.absolute;
      XDG_DATA_HOME = homes.data.absolute;
      XDG_STATE_HOME = homes.state.absolute;
    };
    wrapperEnvironment =
      environment
      // {
        SUMI_MANIFEST = manifestPath;
        SUMI_STATE_DIR = stateDirectory;
      };
    wrappedCli = pkgs.symlinkJoin {
      name = "sumi";
      paths = [baseCli];
      nativeBuildInputs = [pkgs.makeWrapper];
      postBuild = ''
        wrapProgram "$out/bin/sumi" ${lib.escapeShellArgs (lib.concatMap (name: ["--set" name (toString wrapperEnvironment.${name})]) (builtins.attrNames wrapperEnvironment))}
      '';
    };
  in {
    assertions = [
      {
        assertion = home != null;
        message = "sumi.homeDirectory must be set";
      }
      {
        assertion = cfg.facet != {};
        message = "sumi.facet must define at least one facet";
      }
      {
        assertion = homeErrors == [];
        message = "invalid Sumi directories: ${lib.concatStringsSep "; " homeErrors}";
      }
      {
        assertion = facetErrors == [];
        message = "invalid Sumi facets: ${lib.concatStringsSep "; " facetErrors}";
      }
      {
        assertion = fileErrors == [];
        message = "invalid Sumi files: ${lib.concatStringsSep "; " fileErrors}";
      }
      {
        assertion = duplicateFiles == [];
        message = "duplicate Sumi files: ${lib.concatStringsSep ", " duplicateFiles}";
      }
      {
        assertion = effectErrors == [];
        message = "invalid Sumi effects: ${lib.concatStringsSep "; " effectErrors}";
      }
    ];

    lib.sumi.paths = {
      inherit home;
      config = homes.config.absolute;
      cache = homes.cache.absolute;
      data = homes.data.absolute;
      state = homes.state.absolute;
      sumiState = stateDirectory;
    };

    environment = {
      variables = lib.mapAttrs (_: lib.mkDefault) environment;
      etc."sumi/manifest.json".source = manifestPath;
      systemPackages = [wrappedCli];
    };

    sumi = {
      generated.manifest = manifestPath;
      package = wrappedCli;
    };
  });
}
