{
  description = "seni: runtime config switching for nix";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
  inputs.nix-darwin = {
    url = "github:nix-darwin/nix-darwin/nix-darwin-26.05";
    inputs.nixpkgs.follows = "nixpkgs";
  };

  outputs = {
    self,
    nix-darwin,
    nixpkgs,
    ...
  }: let
    systems = nixpkgs.lib.systems.flakeExposed;
    forAllSystems = nixpkgs.lib.genAttrs systems;
  in {
    nixosModules.default = import ./nix/nixos.nix;

    darwinModules.default = import ./nix/darwin.nix;

    packages = forAllSystems (system: let
      seni = nixpkgs.legacyPackages.${system}.callPackage ./nix/package.nix {};
    in {
      default = seni;
      inherit seni;
    });

    checks = forAllSystems (system: let
      pkgs = nixpkgs.legacyPackages.${system};
    in
      {
        darwin-module = pkgs.callPackage ./nix/tests/darwin.nix {darwin = nix-darwin;};
        manifest = pkgs.callPackage ./nix/tests/manifest.nix {};
      }
      // nixpkgs.lib.optionalAttrs pkgs.stdenv.hostPlatform.isLinux {
        nixos = pkgs.callPackage ./nix/tests/nixos.nix {};
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
