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
  postActivation = cfg.system.activationScripts.postActivation.text;
in
  assert lib.assertMsg (failedAssertions == []) (lib.concatMapStringsSep "; " (entry: entry.message) failedAssertions);
  assert lib.assertMsg (builtins.hasAttr "seni-activate" cfg.launchd.agents) "Seni's system LaunchAgent was not installed";
  assert lib.assertMsg (!builtins.hasAttr "seni-activate" cfg.launchd.user.agents) "Seni's LaunchAgent requires a primary user";
  assert lib.assertMsg (builtins.hasAttr "org.seni.activate.plist" cfg.environment.launchAgents) "Seni's system LaunchAgent was not materialized";
  assert lib.assertMsg (cfg.launchd.agents.seni-activate.serviceConfig.Label == "org.seni.activate" && cfg.launchd.agents.seni-activate.serviceConfig.RunAtLoad) "Seni's system LaunchAgent is invalid";
  assert lib.assertMsg (toString cfg.launchd.agents.seni-activate.serviceConfig.Program == toString cfg.seni.generated.activation) "Seni's system LaunchAgent uses the wrong activation command";
  assert lib.assertMsg (cfg.system.primaryUser == null && cfg.system.requiresPrimaryUser == []) "Seni unexpectedly requires a primary user";
  assert lib.assertMsg (map lib.getName alicePackages == ["hello" "seni"]) "Alice's Seni packages were not installed";
  assert lib.assertMsg (map lib.getName bobPackages == ["seni"]) "Bob's Seni command was not installed";
  assert lib.assertMsg (cfg.users.users.carol.packages == []) "Carol's disabled Seni packages were installed";
  assert lib.assertMsg (toString aliceCli != toString bobCli) "Seni users share a configured command";
  assert lib.assertMsg (cfg.seni.users.alice.path.home == "/Users/alice") "Alice's home directory was not derived from nix-darwin";
  assert lib.assertMsg (cfg.seni.users.bob.path.home == "/Users/bob") "Bob's home directory was not derived from nix-darwin";
  assert lib.assertMsg (lib.hasInfix "alice" postActivation && lib.hasInfix "bob" postActivation && lib.hasInfix "carol" postActivation) "Seni's LaunchAgents are not restarted for every configured user";
    pkgs.runCommand "seni-darwin-module-test" {} ''
      touch "$out"
    ''
