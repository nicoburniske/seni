{userModule}: {
  config,
  lib,
  options,
  pkgs,
  ...
}: let
  cfg = config.seni;
  inherit (lib) mkOption types;

  userType = types.submoduleWith {
    description = "Seni user configuration";
    class = "seni";
    specialArgs =
      builtins.removeAttrs cfg.specialArgs ["name"]
      // {
        inherit pkgs;
        osConfig = config;
        osOptions = options;
      };
    modules = [./user.nix userModule] ++ cfg.extraModules;
  };
  enabledUsers = lib.filterAttrs (_: user: user.enable) cfg.users;
  users = builtins.attrValues cfg.users;
  stateDirectories = lib.mapAttrs (_: user: "${user.path.state}/seni") cfg.users;

  userPackages = lib.mapAttrs (name: user:
    pkgs.writeShellScriptBin "seni" ''
      export HOME=${lib.escapeShellArg user.path.home}
      . ${lib.escapeShellArg (toString user.environment.loadEnv)}
      export SENI_MANIFEST=${lib.escapeShellArg (toString user.generated.manifest)}
      export SENI_STATE_DIR=${lib.escapeShellArg stateDirectories.${name}}
      exec ${cfg.cli.package}/bin/seni "$@"
    '')
  enabledUsers;

  activation = pkgs.writeShellScript "seni-activate" ''
    user="$(${pkgs.coreutils}/bin/id -un)"
    case "$user" in
      ${lib.concatMapAttrsStringSep "\n" (name: user: let
        command =
          if user.enable
          then "${userPackages.${name}}/bin/seni activate"
          else "${cfg.cli.package}/bin/seni --state-dir ${lib.escapeShellArg stateDirectories.${name}} deactivate";
      in ''
        ${lib.escapeShellArg name})
          exec ${command}
          ;;
      '')
      cfg.users}
      *) exit 0 ;;
    esac
  '';
in {
  options.seni = {
    users = mkOption {
      type = types.attrsWith {
        elemType = userType;
        placeholder = "username";
      };
      default = {};
      description = "Seni user configurations";
    };

    extraModules = mkOption {
      type = types.listOf types.deferredModule;
      default = [];
      description = "modules evaluated in every Seni user configuration";
    };

    specialArgs = mkOption {
      type = types.attrs;
      default = {};
      description = "arguments passed to every Seni user module";
    };

    cli.package = mkOption {
      type = types.package;
      default = pkgs.callPackage ./package.nix {};
      defaultText = lib.literalExpression "pkgs.callPackage ./package.nix {}";
      description = "unconfigured Seni CLI package";
    };

    generated.activation = mkOption {
      type = types.path;
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
      ++ lib.pipe cfg.users [
        (lib.mapAttrsToList (name: user:
          map (assertion:
            assertion
            // {
              message = "Seni user '${name}': ${assertion.message}";
            })
          user.assertions))
        lib.concatLists
      ]
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

    warnings = lib.pipe cfg.users [
      (lib.mapAttrsToList (name: user:
          map (warning: "Seni user '${name}': ${warning}") user.warnings))
      lib.concatLists
    ];

    users.users =
      lib.mapAttrs (name: user: {
        packages = user.packages ++ [userPackages.${name}];
      })
      enabledUsers;

    seni.generated.activation = activation;
  };
}
