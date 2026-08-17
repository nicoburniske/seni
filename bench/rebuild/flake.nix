{
  description = "Seni, Hjem, and Home Manager rebuild benchmark";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    home-manager = {
      url = "github:nix-community/home-manager";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    hjem = {
      url = "github:feel-co/hjem";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = inputs: let
    systems = ["aarch64-linux" "x86_64-linux"];
    benchmark = {
      implementation,
      nonce,
      system,
    }: let
      pkgs = inputs.nixpkgs.legacyPackages.${system};
      files = builtins.genList (index: "theme-${toString index}.conf") 100;
      benchmarkCase = builtins.getEnv "SENI_BENCHMARK_CASE";
      renderTheme = variant: file:
        ''
          theme=${variant}
          file=${file}
        ''
        + pkgs.lib.optionalString (benchmarkCase == "all" || (benchmarkCase == "one" && file == builtins.head files)) ''
          benchmark-revision=${nonce}
        '';
      implementationModule =
        if implementation == "home-manager"
        then {
          imports = [inputs.home-manager.nixosModules.home-manager];

          home-manager = {
            useGlobalPkgs = true;
            useUserPackages = true;
            users.benchmark = {lib, ...}: {
              home.stateVersion = "26.05";

              xdg.configFile = pkgs.lib.genAttrs files (file: {
                text = renderTheme "light" file;
              });

              specialisation.dark.configuration = {
                xdg.configFile = pkgs.lib.genAttrs files (file: {
                  text = lib.mkForce (renderTheme "dark" file);
                });
              };
            };
          };
        }
        else if implementation == "hjem"
        then {
          imports = [inputs.hjem.nixosModules.default];

          hjem.users.benchmark = {
            enable = true;
            xdg.config.files = pkgs.lib.genAttrs files (file: {
              text = renderTheme "light" file;
            });
          };
        }
        else if implementation == "seni"
        then {
          imports = [(import ../../nix/nixos.nix)];

          seni.users.benchmark = {
            facet.theme = {
              default = "light";
              variants = {
                light = "light";
                dark = "dark";
              };
            };

            file.config = pkgs.lib.genAttrs files (file: {
              facet = "theme";
              value = {theme}: renderTheme theme.value file;
            });
          };
        }
        else throw "unknown benchmark implementation '${implementation}'";
    in
      (inputs.nixpkgs.lib.nixosSystem {
        inherit system;
        modules = [
          {
            boot.isContainer = true;
            documentation.enable = false;
            environment.defaultPackages = [];
            networking.hostName = "benchmark";
            system.stateVersion = "26.05";

            users.users.benchmark = {
              createHome = true;
              isNormalUser = true;
              home = "/home/benchmark";
            };
          }
          implementationModule
        ];
      }).config.system.build.toplevel;
  in {
    packages = inputs.nixpkgs.lib.genAttrs systems (system: let
      nonce = builtins.getEnv "SENI_BENCHMARK_REVISION";
    in {
      home-manager = benchmark {
        inherit nonce system;
        implementation = "home-manager";
      };
      hjem = benchmark {
        inherit nonce system;
        implementation = "hjem";
      };
      seni = benchmark {
        inherit nonce system;
        implementation = "seni";
      };
    });
  };
}
