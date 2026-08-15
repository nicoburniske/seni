{
  config,
  lib,
  options,
  pkgs,
  ...
}: let
  cfg = config.seni;
  osConfig = config;

  userType = lib.types.submoduleWith {
    description = "Seni user configuration";
    class = "seni";
    specialArgs =
      builtins.removeAttrs cfg.specialArgs ["name"]
      // {
        inherit osConfig pkgs;
        osOptions = options;
      };
    modules =
      [
        ./user.nix
        ({name, ...}: let
          user = osConfig.users.users.${name} or null;
        in {
          path.home =
            if user == null
            then "/var/empty"
            else user.home;

          assertions = [
            {
              assertion = user != null && user.enable && user.isNormalUser && user.name == name;
              message = "must reference an enabled normal NixOS user whose name matches the profile key";
            }
          ];
        })
      ]
      ++ cfg.extraModules;
  };
in {
  imports = [(import ./module.nix {inherit userType;})];

  config = lib.mkIf (cfg.users != {}) {
    systemd.services = lib.mapAttrs' (name: user:
      lib.nameValuePair "seni-${name}" {
        description = "Activate Seni for ${name}";
        before = ["systemd-user-sessions.service"];
        wantedBy = ["multi-user.target"];
        unitConfig.RequiresMountsFor = [user.path.home user.path.state];
        serviceConfig = {
          ExecStart = cfg.generated.activation;
          RemainAfterExit = true;
          Type = "oneshot";
          User = name;
        };
      })
    cfg.users;
  };
}
