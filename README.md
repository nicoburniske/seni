<div align="center">
  <h1>seni</h1>

  <p>
    <strong>遷移</strong>（せんい）— transition
  </p>
</div>

seni is a small nix-native home environment manager with instant runtime configuration switching for nixos and nix-darwin

it manages files, packages, environment variables, and effects independently for multiple users

in seni, a facet is a runtime setting with named variants. a `theme` facet might have `dark` and `light` variants. files can be derived from the selected variant, and effects can react when it changes

a minimal nixos flake:

```nix
{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    seni = {
      url = "github:nicoburniske/seni";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = {
    nixpkgs,
    seni,
    ...
  }: {
    nixosConfigurations.example = nixpkgs.lib.nixosSystem {
      system = "x86_64-linux";
      modules = [
        ./hardware-configuration.nix
        seni.nixosModules.default
        ({pkgs, ...}: {
          users.users.nico = {
            isNormalUser = true;
            createHome = true;
          };

          seni.users.nico = {
            packages = [pkgs.helix];

            facet.theme = {
              default = "dark";
              variants = {
                dark = "gruvbox";
                light = "modus_operandi";
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
          system.stateVersion = "26.05";
        })
      ];
    };
  };
}
```

a file can also depend on several facets: `facet = ["theme" "density"]`

facet switching is done via the CLI:

```console
seni switch theme=light
```

## alternatives

### [home manager](https://github.com/nix-community/home-manager)

home manager is huge and SLOW: about 163,000 lines before nixpkgs. its experimental specialisations [build a complete home configuration for every variant](https://github.com/nix-community/home-manager/blob/5bd505963717a894b02a57cdbcc00db28d9b029f/modules/misc/specialisation.nix#L12-L45), multiplying evaluation and build time

the payoff is a vast module ecosystem: thousands of programs and services are ready to configure

### [hjem](https://github.com/feel-co/hjem)

hjem and seni are similar in size/complexity

hjem manages files, packages, and session variables, and its standalone CLI works without nix

it has no equivalent to facets or effects. configuration changes require a new generation

### seni

seni is nix-native and has no standalone mode. it prioritizes simplicity and performance: few primitives, no hidden machinery, minimal build-time overhead, and instant facet switching

in the [rebuild benchmark](bench/rebuild/README.md), seni is at least as fast as hjem while home manager takes 1.2–3.6× as long depending on the rebuild case (in a best-case file-only config)
