{
  lib,
  pkgs,
}: let
  fixture = ./manifest.nu;
  eval = lib.evalModules {
    specialArgs = {inherit pkgs;};
    modules = [
      ../default.nix
      {
        options = {
          assertions = lib.mkOption {
            type = lib.types.listOf lib.types.anything;
            default = [];
          };

          environment = lib.mkOption {
            type = lib.types.attrs;
            default = {};
          };
        };

        config.seni = {
          enable = true;
          path.home = "/home/tester";

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
      }
    ];
  };
  failedAssertions = lib.filter (entry: !entry.assertion) eval.config.assertions;
  manifest = eval.config.seni.generated.manifest;
in
  assert lib.assertMsg (failedAssertions == []) (lib.concatMapStringsSep "; " (entry: entry.message) failedAssertions);
    pkgs.runCommand "seni-manifest-test" {
      nativeBuildInputs = [pkgs.nushell];
    } ''
      set -eu
      ${pkgs.nushell}/bin/nu ${./manifest.nu} ${lib.escapeShellArg (toString manifest)} ${lib.escapeShellArg (toString fixture)}
      touch "$out"
    ''
