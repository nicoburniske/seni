def main [
  --runs: int = 3
] {
  if $runs < 1 {
    error make {msg: "--runs must be at least 1"}
  }

  let directory = $env.FILE_PWD
  let flake = $directory | path expand

  mut results = []
  for benchmark_case in [clean one all] {
    for run in 1..$runs {
      let implementations = match ($run mod 3) {
        1 => [home-manager hjem seni]
        2 => [seni home-manager hjem]
        _ => [hjem seni home-manager]
      }

      for implementation in $implementations {
        let store_root = ^mktemp -d | str trim
        let store = $'local?root=($store_root)'
        let target = $'($flake)#($implementation)'
        let build = {|case, revision|
          with-env {
            SENI_BENCHMARK_CASE: $case
            SENI_BENCHMARK_REVISION: $revision
          } {
            do {
              ^nix build --impure --no-link --option eval-cache false --store $store $target
            } | complete
          }
        }

        if $benchmark_case != clean {
          let baseline = do $build baseline baseline
          if $baseline.exit_code != 0 {
            ^chmod -R u+w $store_root
            rm -r $store_root
            print --stderr $baseline.stderr
            error make {msg: $'($implementation) baseline build failed'}
          }
        }

        print $'($benchmark_case) ($run)/($runs): ($implementation)'
        let revision = $'(date now | into int)-($benchmark_case)-($run)-($implementation)'
        let started = date now
        let result = do $build $benchmark_case $revision
        let seconds = ((date now) - $started) / 1sec
        ^chmod -R u+w $store_root
        rm -r $store_root

        if $result.exit_code != 0 {
          print --stderr $result.stderr
          error make {msg: $'($implementation) ($benchmark_case) build failed'}
        }

        $results = $results | append {
          case: $benchmark_case
          implementation: $implementation
          run: $run
          seconds: $seconds
        }
      }
    }
  }

  print ""
  print "raw results"
  print ($results | table)
  print ""
  print "summary"
  print ($results | group-by case implementation | transpose case implementations | each {|case|
    $case.implementations | transpose implementation samples | each {|row|
      {
        case: $case.case
        implementation: $row.implementation
        runs: ($row.samples | length)
        mean_seconds: ($row.samples.seconds | math avg | math round --precision 3)
      }
    }
  } | flatten | sort-by case implementation | table)
}
