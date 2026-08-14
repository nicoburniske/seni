def assert-equal [actual expected message: string] {
  if $actual != $expected {
    error make {msg: $"($message): expected ($expected), got ($actual)"}
  }
}

def assert-true [condition: bool message: string] {
  if not $condition {
    error make {msg: $message}
  }
}

def main [manifest_path: string source_path: string] {
  let manifest = open $manifest_path
  let raw_manifest = open --raw $manifest_path

  assert-equal $manifest.version 4 "manifest version"
  assert-equal $manifest.home "/home/tester" "manifest home"
  assert-equal ($manifest.facets | columns) [theme] "manifest facets"
  assert-equal $manifest.facets.theme.default "light" "facet default"
  assert-equal ($manifest.facets.theme.variants | columns | sort) [dark light] "facet variants"

  assert-equal ($manifest.files | columns | sort) [
    ".config/demo/dynamic-source.nu"
    ".config/demo/dynamic.txt"
    ".config/demo/static-source.nu"
    ".config/demo/static.txt"
  ] "managed files"
  assert-equal $manifest.files.".config/demo/dynamic-source.nu" {facet: theme} "dynamic source"
  assert-equal $manifest.files.".config/demo/dynamic.txt" {facet: theme} "dynamic text"

  let static_text = $manifest.files.".config/demo/static.txt"
  assert-equal (open --raw $static_text) "static" "static text contents"

  let static_source = $manifest.files.".config/demo/static-source.nu"
  assert-true ($static_source != $source_path) "static source was not materialized"
  assert-true ($static_source | path exists) "materialized static source is missing"

  for variant in [dark light] {
    let root = $manifest.facets.theme.variants | get $variant
    assert-true ($root | path exists) $"($variant) variant root is missing"
    assert-equal (open --raw ($root | path join ".config/demo/dynamic.txt")) $"tone=($variant)" $"($variant) dynamic text"

    let source = $root | path join ".config/demo/dynamic-source.nu"
    let target = ^readlink $source | str trim
    assert-true ($target != $source_path) $"($variant) dynamic source was not materialized"
    assert-true ($target | path exists) $"($variant) dynamic source target is missing"
  }

  assert-equal ($manifest.effects | columns | sort) [dynamic static] "effects"
  assert-equal $manifest.effects.static.on [] "static effect filter"
  assert-equal ($manifest.effects.static.exec | first) "/bin/echo" "static effect executable"
  let static_argument = $manifest.effects.static.exec | get 1
  assert-true ($static_argument != $source_path) "static effect argument was not materialized"
  assert-true ($static_argument | path exists) "static effect argument is missing"

  assert-equal $manifest.effects.dynamic.on [theme] "dynamic effect filter"
  assert-equal $manifest.effects.dynamic.exec.facet "theme" "dynamic effect facet"
  for command in ($manifest.effects.dynamic.exec.variants | values) {
    assert-equal ($command | first) "/bin/echo" "dynamic effect executable"
    let argument = $command | get 1
    assert-true ($argument != $source_path) "dynamic effect argument was not materialized"
    assert-true ($argument | path exists) "dynamic effect argument is missing"
  }

  assert-true (not ($raw_manifest | str contains $source_path)) "manifest contains the original source path"
}
