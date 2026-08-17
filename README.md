# seni

<p align="center">
  <strong>遷移 · せんい · transition</strong><br>
  <em>shift happens.</em>
</p>

seni is a small nix-native home environment manager for nixos and nix-darwin

it manages files, packages, environment variables, and effects independently for multiple users

in seni, a facet is a runtime setting with named variants. a `theme` facet might have `dark` and `light` variants. files can be derived from the selected variant, and effects can react when it changes

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
    value = {theme}: ''
      # seni variant: ${theme.variant}
      theme = "${theme.value}"
    '';
    effect = {
      # live reload helix
      exec = ["${pkgs.procps}/bin/pkill" "-USR1" "hx"];
      ignoreFailure = true;
    };
  };
};
```

a file can also depend on several facets: `facet = ["theme" "density"]`

facet switching is done via the CLI:

```console
seni switch theme=light
```


## comparison

### [home manager](https://github.com/nix-community/home-manager)

home manager is huge and SLOW: about 163,000 lines before nixpkgs. its experimental specialisations [build a complete home configuration for every variant](https://github.com/nix-community/home-manager/blob/5bd505963717a894b02a57cdbcc00db28d9b029f/modules/misc/specialisation.nix#L12-L45), multiplying evaluation and build time

the payoff is a vast module ecosystem: thousands of programs and services are ready to configure

### [hjem](https://github.com/feel-co/hjem)

hjem and seni are similar in size/complexity

hjem manages files, packages, and session variables, and its standalone CLI works without nix.

it has no equivalent to facets or effects. configuration changes require a new generation

### seni

seni is nix-native and requires nixos or nix-darwin. it has no standalone mode

seni prioritizes simplicity and performance: few primitives, no hidden machinery, minimal build-time overhead, and instant facet switching
