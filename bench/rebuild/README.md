# rebuild benchmark

compares complete nixos system builds with 100 distinct theme-dependent files

- home manager builds 100 light files and 100 dark specialisation files
- seni builds 100 files for both variants of a `theme` facet
- hjem builds 100 static light-theme files since it has no runtime variants

this is a favorable case for home manager: it enables no program or service modules beyond the files and specialisation being measured. real module-heavy configurations can easily reach minute-scale incremental rebuilds, but this benchmark does not measure that workload

cases:

- `clean` builds the unchanged configuration in a new temporary nix store
- `one` builds a baseline then changes one file
- `all` builds a baseline then changes all 100 files

run three samples per implementation:

```console
nu run.nu
```

## results

| implementation |   clean | change one | change all |
| -------------- | ------: | ---------: | ---------: |
| home manager   | 29.37 s |     4.78 s |    12.58 s |
| hjem           | 32.22 s |     3.57 s |     6.64 s |
| seni           | 24.81 s |     3.48 s |     3.51 s |

results are means of three runs. each sample uses its own store with no warmup and the runner prints every raw timing before the summary
