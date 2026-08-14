{
  description = "Sumi: facet-based runtime config switching for NixOS and nix-darwin";

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

      config = lib.mkIf config.sumi.enable {
        system.userActivationScripts.sumi = ''
          if [ "$HOME" = ${lib.escapeShellArg config.sumi.path.home} ]; then
            ${config.sumi.package}/bin/sumi activate
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

      config = lib.mkIf config.sumi.enable {
        assertions = [
          {
            assertion = config.system.primaryUser == null || config.sumi.path.home == config.system.primaryUserHome;
            message = "sumi.path.home must be the nix-darwin primary user's home";
          }
        ];

        system.requiresPrimaryUser = ["sumi"];
        system.activationScripts.postActivation.text = lib.mkAfter ''
          /usr/bin/sudo -H -u ${lib.escapeShellArg config.system.primaryUser} -- ${config.sumi.package}/bin/sumi activate
        '';
      };
    };

    packages = forAllSystems (system: let
      sumi = nixpkgs.legacyPackages.${system}.callPackage ./cli/default.nix {};
    in {
      default = sumi;
      inherit sumi;
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
        program = "${self.packages.${system}.sumi}/bin/sumi";
      };
    });
  };
}
