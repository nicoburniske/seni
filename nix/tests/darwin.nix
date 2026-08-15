{
  darwin,
  lib,
  pkgs,
}: let
  eval = darwin.lib.darwinSystem {
    system = "aarch64-darwin";
    modules = [
      ../darwin.nix
      ({pkgs, ...}: {
        system.stateVersion = 6;

        users.users = {
          alice = {
            uid = 501;
            home = "/Users/alice";
          };
          bob = {
            uid = 502;
            home = "/Users/bob";
          };
          carol = {
            uid = 503;
            home = "/Users/carol";
          };
        };

        seni.users = {
          alice = {
            facet.theme = {
              default = "light";
              variants.light = "light";
            };
            packages = [pkgs.hello];
          };
          bob.facet.theme = {
            default = "dark";
            variants.dark = "dark";
          };
          carol = {
            enable = false;
            packages = [pkgs.hello];
          };
        };
      })
    ];
  };

  cfg = eval.config;
  failedAssertions = lib.filter (entry: !entry.assertion) cfg.assertions;
  alicePackages = cfg.users.users.alice.packages;
  bobPackages = cfg.users.users.bob.packages;
  aliceCli = lib.last alicePackages;
  bobCli = lib.last bobPackages;
  agent = cfg.launchd.agents.seni-activate.serviceConfig;
  postActivation = cfg.system.activationScripts.postActivation.text;
in
  assert lib.assertMsg (failedAssertions == []) (lib.concatMapStringsSep "; " (entry: entry.message) failedAssertions);
  assert lib.assertMsg (builtins.hasAttr "seni-activate" cfg.launchd.agents && !builtins.hasAttr "seni-activate" cfg.launchd.user.agents) "Seni's global LaunchAgent was not installed";
  assert lib.assertMsg (agent.Label == "org.seni.activate" && agent.RunAtLoad && agent.Program == toString cfg.seni.generated.activation) "Seni's global LaunchAgent is invalid";
  assert lib.assertMsg (cfg.system.primaryUser == null && cfg.system.requiresPrimaryUser == []) "Seni unexpectedly requires a primary user";
  assert lib.assertMsg (map lib.getName alicePackages == ["hello" "seni"] && map lib.getName bobPackages == ["seni"] && toString aliceCli != toString bobCli) "Seni's user commands were not installed separately";
  assert lib.assertMsg (cfg.users.users.carol.packages == []) "Carol's disabled Seni packages were installed";
  assert lib.assertMsg (cfg.seni.users.alice.path.home == "/Users/alice" && cfg.seni.users.bob.path.home == "/Users/bob") "Seni's home directories were not derived from nix-darwin";
  assert lib.assertMsg (lib.hasInfix "alice" postActivation && lib.hasInfix "bob" postActivation && lib.hasInfix "launchctl kickstart" postActivation) "Seni's enabled users are not activated after rebuilds";
  assert lib.assertMsg (lib.hasInfix "/usr/bin/sudo --user=carol -- " postActivation) "Seni's disabled users are not deactivated after rebuilds";
    pkgs.runCommand "seni-darwin-module-test" {} ''
      touch "$out"
    ''
