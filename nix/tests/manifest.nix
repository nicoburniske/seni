{
  lib,
  pkgs,
}: let
  fixture = ./manifest.nu;

  profile = {
    facet.theme = {
      default = "light";
      variants = {
        light = {
          asset = fixture;
          tone = "light";
        };
        dark = {
          asset = fixture;
          tone = "dark";
        };
      };
    };

    file.config = {
      "demo/static.txt".value = "static";
      "demo/static-source.nu".value = fixture;
      "demo/dynamic.txt" = {
        facet = "theme";
        value = context: "tone=${context.value.tone}";
      };
      "demo/dynamic-source.nu" = {
        facet = "theme";
        value = context: context.value.asset;
      };
    };

    effect = {
      static = {
        exec = ["/bin/echo" fixture];
        ignoreFailure = true;
      };
      dynamic = {
        on = ["theme"];
        exec = context: ["/bin/echo" context.value.asset];
      };
    };
  };

  eval = lib.evalModules {
    specialArgs = {inherit pkgs;};
    modules = [
      ../nixos.nix
      {
        options = {
          assertions = lib.mkOption {
            type = lib.types.listOf lib.types.anything;
            default = [];
          };

          warnings = lib.mkOption {
            type = lib.types.listOf lib.types.str;
            default = [];
          };

          systemd.services = lib.mkOption {
            type = lib.types.attrsOf lib.types.anything;
            default = {};
          };

          users.users = lib.mkOption {
            type = lib.types.attrsOf (lib.types.submodule ({name, ...}: {
              options = {
                enable = lib.mkOption {
                  type = lib.types.bool;
                  default = true;
                };
                home = lib.mkOption {type = lib.types.str;};
                isNormalUser = lib.mkOption {
                  type = lib.types.bool;
                  default = false;
                };
                name = lib.mkOption {
                  type = lib.types.str;
                  default = name;
                };
                packages = lib.mkOption {
                  type = lib.types.listOf lib.types.package;
                  default = [];
                };
              };
            }));
            default = {};
          };
        };

        config = {
          users.users = {
            alice = {
              home = "/home/alice";
              isNormalUser = true;
            };
            bob = {
              home = "/home/bob";
              isNormalUser = true;
            };
            carol = {
              home = "/home/carol";
              isNormalUser = true;
            };
          };

          seni = {
            specialArgs.extraText = "extra";
            extraModules = [
              ({extraText, ...}: {
                file.home."extra.txt".value = extraText;
              })
            ];
            users = {
              alice = profile // {packages = [pkgs.hello];};
              bob =
                profile
                // {
                  facet.theme = profile.facet.theme // {default = "dark";};
                };
              carol = {
                enable = false;
                packages = [pkgs.hello];
              };
            };
          };
        };
      }
    ];
  };

  failedAssertions = lib.filter (entry: !entry.assertion) eval.config.assertions;
  alice = eval.config.seni.users.alice.generated.manifest;
  bob = eval.config.seni.users.bob.generated.manifest;
  alicePackages = eval.config.users.users.alice.packages;
  bobPackages = eval.config.users.users.bob.packages;
  aliceCli = lib.last alicePackages;
  bobCli = lib.last bobPackages;
in
  assert lib.assertMsg (failedAssertions == []) (lib.concatMapStringsSep "; " (entry: entry.message) failedAssertions);
  assert lib.assertMsg (lib.length alicePackages == 2 && builtins.head alicePackages == pkgs.hello) "Alice's Seni packages were not installed";
  assert lib.assertMsg (lib.length bobPackages == 1) "Bob's Seni command was not installed";
  assert lib.assertMsg (eval.config.users.users.carol.packages == []) "Carol's disabled Seni packages were installed";
  assert lib.assertMsg (builtins.attrNames eval.config.systemd.services == ["seni-alice" "seni-bob" "seni-carol"]) "Seni's per-user activation services were not installed";
    pkgs.runCommand "seni-manifest-test" {
      nativeBuildInputs = [pkgs.bash pkgs.nushell];
    } ''
      set -eu
      ${pkgs.nushell}/bin/nu ${./manifest.nu} ${lib.escapeShellArg (toString alice)} ${lib.escapeShellArg (toString fixture)} /home/alice light
      ${pkgs.nushell}/bin/nu ${./manifest.nu} ${lib.escapeShellArg (toString bob)} ${lib.escapeShellArg (toString fixture)} /home/bob dark
      ${pkgs.bash}/bin/bash -n ${aliceCli}/bin/seni
      ${pkgs.bash}/bin/bash -n ${bobCli}/bin/seni
      ${pkgs.bash}/bin/bash -n ${eval.config.seni.generated.activation}
      touch "$out"
    ''
