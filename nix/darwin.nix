{
  config,
  lib,
  pkgs,
  ...
}: let
  cfg = config.seni;
  applicationsCfg = cfg.darwin.applications;

  userApplications = lib.mapAttrs (name: user:
    pkgs.buildEnv {
      name = "seni-${name}-applications";
      paths = lib.optionals user.enable user.packages;
      pathsToLink = ["/Applications"];
    })
  cfg.users;

  darwinActivation = pkgs.writeShellScript "seni-darwin-activate" ''
    set -eu

    user="$(${pkgs.coreutils}/bin/id -un)"
    case "$user" in
      ${lib.concatMapAttrsStringSep "\n" (name: user: let
        target = "${user.path.home}/${applicationsCfg.directory}";
      in ''
        ${lib.escapeShellArg name})
          target=${lib.escapeShellArg target}
          ${pkgs.coreutils}/bin/mkdir -p "$target"
          ${pkgs.rsync}/bin/rsync \
            --recursive \
            --checksum \
            --perms \
            --links \
            --copy-unsafe-links \
            --specials \
            --delete \
            --chmod=+w \
            ${userApplications.${name}}/Applications/ \
            "$target/"
          exec ${cfg.generated.activation}
          ;;
      '')
      cfg.users}
      *) exit 0 ;;
    esac
  '';
in {
  imports = [
    (import ./module.nix {
      userModule = {
        name,
        osConfig,
        ...
      }: let
        user = osConfig.users.users.${name} or null;
      in {
        path.home =
          if user == null || user.home == null
          then "/var/empty"
          else toString user.home;

        assertions = [
          {
            assertion = user != null && user.home != null && user.name == name;
            message = "must reference a nix-darwin user with a home directory whose name matches the profile key";
          }
        ];
      };
    })
  ];

  options.seni = {
    darwin.applications = {
      directory = lib.mkOption {
        type = lib.types.str;
        default = "Applications/Seni Apps";
        description = "application directory relative to each user's home directory";
      };
    };

    generated = {
      applications = lib.mkOption {
        type = lib.types.attrsOf lib.types.package;
        readOnly = true;
        internal = true;
      };

      darwinActivation = lib.mkOption {
        type = lib.types.path;
        readOnly = true;
        internal = true;
      };
    };
  };

  config = lib.mkIf (cfg.users != {}) {
    assertions = [
      {
        assertion =
          applicationsCfg.directory
          != ""
          && !lib.hasPrefix "/" applicationsCfg.directory
          && lib.all (segment: segment != "" && segment != "." && segment != "..") (lib.splitString "/" applicationsCfg.directory);
        message = "seni.darwin.applications.directory must be a normalized relative path";
      }
    ];

    seni.generated = {
      applications = userApplications;
      inherit darwinActivation;
    };

    launchd.agents.seni-activate.serviceConfig = {
      Label = "org.seni.activate";
      Program = toString darwinActivation;
      RunAtLoad = true;
    };

    system.activationScripts.postActivation.text = lib.mkAfter (
      lib.concatMapAttrsStringSep "\n" (name: user: let
        escapedName = lib.escapeShellArg name;
      in
        if user.enable
        then ''
          if uid="$(${pkgs.coreutils}/bin/id -u ${escapedName} 2>/dev/null)"; then
            /bin/launchctl kickstart -k "gui/$uid/org.seni.activate" 2>/dev/null || true
          fi
        ''
        else ''
          /usr/bin/sudo --user=${escapedName} -- ${darwinActivation}
        '')
      cfg.users
    );
  };
}
