def assert-true [condition: bool message: string] {
  if not $condition {
    error make {msg: $message}
  }
}

def main [manifest_path: string source_path: string] {
  let manifest = open $manifest_path
  let raw_manifest = open --raw $manifest_path

  let roots = ($manifest | get facets.theme.variants)
  assert-true (($roots.light | path exists)) "light variant root is missing"
  assert-true (($roots.dark | path exists)) "dark variant root is missing"

  let static_source = ($manifest | get files.".config/demo/static-source.txt")
  assert-true ($static_source != $source_path) "static file source was not materialized"
  assert-true (($static_source | path exists)) "materialized static file source is missing"
  let static_contents = open --raw $static_source
  assert-true (($static_contents | str length) > 0) "static file source is empty"

  for root in ($roots | transpose variant path) {
    let generated = ($root.path | path join ".config/demo/asset-path.txt")
    assert-true (($generated | path exists)) $"generated file is missing: ($generated)"

    let contents = open --raw $generated
    assert-true (($contents | str contains "/nix/store/")) "generated file did not use a materialized facet asset"
    assert-true (not ($contents | str contains $source_path)) "generated file still uses the original source path"
  }

  let effect_cases = ($manifest | get effects.asset-path.exec.variants)
  for case in ($effect_cases | transpose variant argv) {
    let asset = ($case.argv | get 1)
    assert-true (($asset | str contains "/nix/store/")) "effect command did not use a materialized facet asset"
    assert-true (not ($asset | str contains $source_path)) "effect command still uses the original source path"
  }

  assert-true (not ($raw_manifest | str contains $source_path)) "manifest still contains the original source path"
}
