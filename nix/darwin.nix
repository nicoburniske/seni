{
  config,
  lib,
  pkgs,
  ...
}: let
  cfg = config.seni;
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

  config = lib.mkIf (cfg.users != {}) {
    launchd.agents.seni-activate.serviceConfig = {
      Label = "org.seni.activate";
      Program = toString cfg.generated.activation;
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
          /usr/bin/sudo --user=${escapedName} -- ${cfg.generated.activation}
        '')
      cfg.users
    );
  };
}
