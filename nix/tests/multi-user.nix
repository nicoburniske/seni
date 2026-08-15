{pkgs}:
pkgs.testers.nixosTest {
  name = "seni-multi-user";

  nodes.machine = {
    lib,
    pkgs,
    ...
  }: {
    imports = [../nixos.nix];

    users.users = {
      alice = {
        isNormalUser = true;
        uid = 1000;
        createHome = true;
      };
      bob = {
        isNormalUser = true;
        uid = 1001;
        createHome = true;
      };
      carol = {
        isNormalUser = true;
        uid = 1002;
        createHome = true;
      };
    };

    seni = {
      extraModules = [
        ({
          name,
          pkgs,
          ...
        }: {
          facet.theme.variants = {
            light = "light";
            dark = "dark";
          };

          file.home = {
            ".seni-name".value = name;
            ".seni-theme" = {
              facet = "theme";
              value = {value, ...}: value;
            };
          };

          effect.record-user.exec = [
            (pkgs.writeShellScript "seni-record-user" ''
              ${pkgs.coreutils}/bin/id -un > "$HOME/.seni-activated-by"
            '')
          ];
        })
      ];

      users = {
        alice = {
          facet.theme.default = "light";
          packages = [pkgs.hello];
        };
        bob.facet.theme.default = "dark";
      };
    };

    specialisation.alice-disabled.configuration.seni.users.alice.enable = lib.mkForce false;

    system.stateVersion = "26.05";
  };

  testScript = ''
    start_all()
    machine.wait_for_unit("multi-user.target")
    machine.succeed("systemctl is-active seni-alice.service")
    machine.succeed("systemctl is-active seni-bob.service")
    machine.fail("systemctl is-active user@1000.service")
    machine.fail("systemctl is-active user@1001.service")

    machine.wait_until_succeeds("test -L /home/alice/.seni-theme")
    machine.wait_until_succeeds("test -L /home/bob/.seni-theme")
    machine.succeed("test $(cat /home/alice/.seni-name) = alice")
    machine.succeed("test $(cat /home/bob/.seni-name) = bob")
    machine.succeed("test $(cat /home/alice/.seni-theme) = light")
    machine.succeed("test $(cat /home/bob/.seni-theme) = dark")
    machine.succeed("test $(cat /home/alice/.seni-activated-by) = alice")
    machine.succeed("test $(cat /home/bob/.seni-activated-by) = bob")
    machine.succeed("test $(stat -c %U /home/alice/.local/state/seni) = alice")
    machine.succeed("test $(stat -c %U /home/bob/.local/state/seni) = bob")
    machine.succeed("test -x /etc/profiles/per-user/alice/bin/hello")
    machine.succeed("test -x /etc/profiles/per-user/alice/bin/seni")
    machine.succeed("test -x /etc/profiles/per-user/bob/bin/seni")
    machine.fail("test -e /etc/profiles/per-user/carol/bin/seni")

    machine.succeed("su - alice -c 'seni switch theme=dark'")
    machine.succeed("test $(cat /home/alice/.seni-theme) = dark")
    machine.succeed("test $(cat /home/bob/.seni-theme) = dark")
    machine.fail("su - carol -c 'seni selection'")

    machine.wait_until_succeeds("! systemctl is-active user@1000.service")
    machine.succeed("/run/current-system/specialisation/alice-disabled/bin/switch-to-configuration test")
    machine.wait_until_succeeds("test ! -L /home/alice/.seni-theme")
    machine.succeed("test -L /home/bob/.seni-theme")
    machine.succeed("test $(cat /home/bob/.seni-theme) = dark")
    machine.fail("test -e /etc/profiles/per-user/alice/bin/seni")
    machine.succeed("test -x /etc/profiles/per-user/bob/bin/seni")
    machine.fail("su - alice -c 'seni selection'")
  '';
}
