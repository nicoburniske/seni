{userType}: {
  config,
  lib,
  pkgs,
  ...
}: let
  cfg = config.seni;
  enabledUsers = lib.filterAttrs (_: user: user.enable) cfg.users;
  users = builtins.attrValues cfg.users;
  stateDirectories = lib.mapAttrs (_: user: "${user.path.state}/seni") cfg.users;

  userPackages = lib.mapAttrs (name: user:
    pkgs.writeShellScriptBin "seni" ''
      export HOME=${lib.escapeShellArg user.path.home}
      export XDG_CONFIG_HOME=${lib.escapeShellArg user.path.config}
      export XDG_CACHE_HOME=${lib.escapeShellArg user.path.cache}
      export XDG_DATA_HOME=${lib.escapeShellArg user.path.data}
      export XDG_STATE_HOME=${lib.escapeShellArg user.path.state}
      export SENI_MANIFEST=${lib.escapeShellArg (toString user.generated.manifest)}
      export SENI_STATE_DIR=${lib.escapeShellArg stateDirectories.${name}}
      exec ${cfg.cli.package}/bin/seni "$@"
    '')
  enabledUsers;

  activation = pkgs.writeShellScript "seni-activate" ''
    user="$(${pkgs.coreutils}/bin/id -un)"
    case "$user" in
      ${lib.concatMapAttrsStringSep "\n" (name: user:
      if user.enable
      then ''
        ${lib.escapeShellArg name})
          exec ${userPackages.${name}}/bin/seni activate
          ;;
      ''
      else ''
        ${lib.escapeShellArg name})
          exec ${cfg.cli.package}/bin/seni --state-dir ${lib.escapeShellArg stateDirectories.${name}} deactivate
          ;;
      '')
    cfg.users}
      *) exit 0 ;;
    esac
  '';
in {
  options.seni = {
    users = lib.mkOption {
      type = lib.types.attrsWith {
        elemType = userType;
        placeholder = "username";
      };
      default = {};
      description = "Seni user configurations";
    };

    extraModules = lib.mkOption {
      type = lib.types.listOf lib.types.deferredModule;
      default = [];
      description = "modules evaluated in every Seni user configuration";
    };

    specialArgs = lib.mkOption {
      type = lib.types.attrs;
      default = {};
      description = "arguments passed to every Seni user module";
    };

    cli.package = lib.mkOption {
      type = lib.types.package;
      default = pkgs.callPackage ./package.nix {};
      defaultText = lib.literalExpression "pkgs.callPackage ./package.nix {}";
      description = "unconfigured Seni CLI package";
    };

    generated.activation = lib.mkOption {
      type = lib.types.path;
      readOnly = true;
      internal = true;
    };
  };

  config = {
    assertions =
      [
        {
          assertion = !(cfg.specialArgs ? name);
          message = "seni.specialArgs.name is reserved";
        }
      ]
      ++ lib.concatLists (lib.mapAttrsToList (name: user:
        map (assertion:
          assertion
          // {
            message = "Seni user '${name}': ${assertion.message}";
          })
        user.assertions)
      cfg.users)
      ++ [
        {
          assertion = lib.allUnique (map (user: user.path.home) users);
          message = "Seni users must have distinct home directories";
        }
        {
          assertion = lib.allUnique (builtins.attrValues stateDirectories);
          message = "Seni users must have distinct state directories";
        }
      ];

    warnings = lib.concatLists (lib.mapAttrsToList (name: user: map (warning: "Seni user '${name}': ${warning}") user.warnings) cfg.users);

    users.users =
      lib.mapAttrs (name: user: {
        packages = user.packages ++ [userPackages.${name}];
      })
      enabledUsers;

    seni.generated.activation = activation;
  };
}
