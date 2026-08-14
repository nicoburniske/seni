{
  description = "seni: runtime config switching for nix";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs = {
    self,
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
    in {
      manifest = pkgs.callPackage ./nix/tests/manifest.nix {};
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
