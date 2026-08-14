{
  config,
  lib,
  ...
}: {
  imports = [./module.nix];

  config = lib.mkIf config.seni.enable {
    assertions = [
      {
        assertion = config.system.primaryUser == null || config.seni.path.home == config.system.primaryUserHome;
        message = "seni.path.home must be the nix-darwin primary user's home";
      }
    ];

    system.requiresPrimaryUser = ["seni"];
    system.activationScripts.postActivation.text = lib.mkAfter ''
      /usr/bin/sudo -H -u ${lib.escapeShellArg config.system.primaryUser} -- ${config.seni.package}/bin/seni activate
    '';
  };
}
