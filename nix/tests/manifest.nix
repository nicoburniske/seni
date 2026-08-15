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
    file.home."extra.txt".value = "extra";

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
    class = "seni";
    specialArgs = {
      inherit pkgs;
      name = "alice";
    };
    modules = [
      ../user.nix
      profile
      {path.home = "/home/alice";}
    ];
  };

  failedAssertions = lib.filter (entry: !entry.assertion) eval.config.assertions;
  manifest = eval.config.generated.manifest;
in
  assert lib.assertMsg (failedAssertions == []) (lib.concatMapStringsSep "; " (entry: entry.message) failedAssertions);
    pkgs.runCommand "seni-manifest-test" {} ''
      ${pkgs.nushell}/bin/nu ${./manifest.nu} ${lib.escapeShellArg (toString manifest)} ${lib.escapeShellArg (toString fixture)} /home/alice light
      touch "$out"
    ''
