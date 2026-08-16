# seni

> shift happens.

遷移 · せんい · transition

seni is a small nix-native home environment manager for nixos and nix-darwin

it manages files, packages, environment variables, and effects independently for multiple users

unlike [home manager](https://github.com/nix-community/home-manager) and [hjem](https://github.com/feel-co/hjem), seni can build multiple variants of a configuration ahead of time and switch between them without rebuilding

a facet represents something that can change at runtime, such as a theme, application profile, or system setting

every variant is built with the system, so switching only changes the active configuration and runs its effects

```nix
seni.users.nico = {
  packages = [pkgs.helix];

  facet.theme = {
    default = "dark";
    variants = {
      dark = "gruvbox";
      light = "github_light";
    };
  };

  file.config."helix/config.toml" = {
    facet = "theme";
    value = {value, ...}: ''
      theme = "${value}"
    '';
    effect = {
      # reload helix config
      exec = ["${pkgs.procps}/bin/pkill" "-USR1" "hx"];
      ignoreFailure = true;
    };
  };
};
```

facet switching is done via the CLI:

```console
seni switch theme=light
```

seni is deliberately small, FAST, and simple. use the compact set of primitives to build your own modules, without any hidden machinery
