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
      "demo/static.txt" = {
        value = "static";
        effect.exec = ["/bin/true"];
      };
      "demo/static-source.nu".value = fixture;
      "demo/dynamic.txt" = {
        facet = "theme";
        value = context: "tone=${context.value.tone}";
        effect = {
          exec = ["/bin/true"];
          ignoreFailure = true;
        };
      };
      "demo/dynamic-source.nu" = {
        facet = "theme";
        value = context: context.value.asset;
      };
    };
    file.home."extra.txt".value = "extra";

    environment.sessionVariables.EDITOR = "hx";

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
      (import ../user.nix {existingFileStrategy = "backup";})
      profile
      {path.home = "/home/alice";}
    ];
  };

  failedAssertions = lib.filter (entry: !entry.assertion) eval.config.assertions;
  stateCollisionAssertions = (eval.extendModules {
    modules = [{file.state."seni/current".value = fixture;}];
  }).config.assertions;
  environment = eval.config.environment;
  manifest = eval.config.generated.manifest;
in
  assert lib.assertMsg (failedAssertions == []) (lib.concatMapStringsSep "; " (entry: entry.message) failedAssertions);
  assert lib.assertMsg (lib.any (entry: !entry.assertion && entry.message == "file destinations cannot overlap the Seni state directory") stateCollisionAssertions) "Seni's state directory was accepted as a file destination";
  assert lib.assertMsg (environment.sessionVariables
    == {
      EDITOR = "hx";
      XDG_CACHE_HOME = "/home/alice/.cache";
      XDG_CONFIG_HOME = "/home/alice/.config";
      XDG_DATA_HOME = "/home/alice/.local/share";
      XDG_STATE_HOME = "/home/alice/.local/state";
    }) "Seni's environment variables were not merged";
    pkgs.runCommand "seni-manifest-test" {} ''
      ${pkgs.nushell}/bin/nu ${fixture} ${lib.escapeShellArg (toString manifest)} ${lib.escapeShellArg (toString fixture)} /home/alice light
      touch "$out"
    ''
