{
  lib,
  pkgs,
}: let
  types = lib.types;

  eval = lib.evalModules {
    specialArgs = {
      inherit pkgs;
    };

    modules = [
      ../default.nix
      ({...}: {
        options = {
          assertions = lib.mkOption {
            type = types.listOf types.anything;
            default = [];
          };

          environment = lib.mkOption {
            type = types.attrs;
            default = {};
          };

          lib = lib.mkOption {
            type = types.attrs;
            default = {};
          };
        };

        config.sumi = {
          enable = true;
          homeDirectory = "/home/tester";

          facet.theme = {
            default = "light";
            variants = {
              light.tone = "light";
              dark.tone = "dark";
            };
          };

          file.config."demo/app.conf" = {
            facet = "theme";
            value = context: "tone=${context.value.tone}";
          };

          effect = {
            demo.exec = ["/bin/echo" "reload"];
            generated = {
              on = ["theme"];
              exec = context: ["/bin/echo" "tone=${context.value.tone}"];
            };
          };
        };
      })
    ];
  };

  manifest = eval.config.sumi.generated.manifest;
in
  pkgs.runCommand "sumi-manifest-shape-test" {
    nativeBuildInputs = [pkgs.nushell];
  } ''
    set -eu

    manifest=${lib.escapeShellArg (toString manifest)}
    test -f "$manifest"

    ${pkgs.nushell}/bin/nu ${./manifest-shape.nu} "$manifest"

    touch "$out"
  ''
