{
  config,
  lib,
  ...
}: {
  imports = [./module.nix];

  config = lib.mkIf config.seni.enable {
    system.userActivationScripts.seni = ''
      if [ "$HOME" = ${lib.escapeShellArg config.seni.path.home} ]; then
        ${config.seni.package}/bin/seni activate
      fi
    '';
  };
}
