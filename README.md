# seni

> shift happens.

遷移 · せんい · transition

seni is a lightweight home-manager replacement for managing user dotfiles with nix

define facets for your config, then switch between their variants without rebuilding

facets can represent themes, system fan curves, or any other setting you want to switch on the fly

```nix
seni = {
  enable = true;
  path.home = "/home/you";

  facet.theme = {
    default = "light";
    variants = {
      light = "light";
      dark = "dark";
    };
  };

  file.config."app/theme" = {
    facet = "theme";
    value = {value, ...}: "theme = ${value}";
  };
};
```

```console
seni switch theme=dark
```
