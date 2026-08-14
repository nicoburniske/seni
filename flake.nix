{
  description = "Seni: facet-based runtime config switching for NixOS and nix-darwin";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs = {
    self,
    nixpkgs,
    ...
  }: let
    systems = nixpkgs.lib.systems.flakeExposed;
    forAllSystems = nixpkgs.lib.genAttrs systems;
  in {
    nixosModules.default = {
      config,
      lib,
      ...
    }: {
      imports = [./default.nix];

      config = lib.mkIf config.seni.enable {
        system.userActivationScripts.seni = ''
          if [ "$HOME" = ${lib.escapeShellArg config.seni.path.home} ]; then
            ${config.seni.package}/bin/seni activate
          fi
        '';
      };
    };

    darwinModules.default = {
      config,
      lib,
      ...
    }: {
      imports = [./default.nix];

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
    };

    packages = forAllSystems (system: let
      seni = nixpkgs.legacyPackages.${system}.callPackage ./cli/default.nix {};
    in {
      default = seni;
      inherit seni;
    });

    checks = forAllSystems (system: let
      pkgs = nixpkgs.legacyPackages.${system};
    in {
      manifest = pkgs.callPackage ./tests/manifest.nix {};
    });

    devShells = forAllSystems (system: let
      pkgs = nixpkgs.legacyPackages.${system};
    in {
      default = pkgs.mkShell {
        packages = with pkgs; [
          gcc
          rustc
          cargo
          clippy
          rustfmt
          rust-analyzer
        ];
      };
    });

    apps = forAllSystems (system: {
      default = {
        type = "app";
        program = "${self.packages.${system}.seni}/bin/seni";
      };
    });
  };
}
