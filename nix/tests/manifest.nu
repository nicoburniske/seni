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

def main [manifest_path: string source_path: string expected_home: string expected_default: string] {
  let manifest = open $manifest_path
  let raw_manifest = open --raw $manifest_path

  assert-equal $manifest.version 1 "manifest version"
  assert-equal $manifest.home $expected_home "manifest home"
  assert-equal $manifest.existingFileStrategy "backup" "existing file strategy"
  assert-equal ($manifest.facets | columns) [density theme] "manifest facets"
  assert-equal $manifest.facets.theme.default $expected_default "facet default"
  assert-equal ($manifest.facets.theme.variants | sort) [dark light] "facet variants"
  assert-equal $manifest.facets.density.default compact "density default"
  assert-equal ($manifest.facets.density.variants | sort) [compact roomy] "density variants"

  assert-equal ($manifest.files | columns | sort) [
    ".config/demo/dynamic-source.nu"
    ".config/demo/dynamic.txt"
    ".config/demo/multi.txt"
    ".config/demo/static-source.nu"
    ".config/demo/static.txt"
    "extra.txt"
  ] "managed files"
  let roots = $manifest.roots | enumerate
  let theme_root = $roots | where item.facets == [theme] | first
  assert-equal $manifest.files.".config/demo/dynamic-source.nu" {root: $theme_root.index} "dynamic source"
  assert-equal $manifest.files.".config/demo/dynamic.txt" {root: $theme_root.index} "dynamic text"

  let multi_root = $roots | where item.facets == [density theme] | first
  assert-equal ($multi_root.item.variants | length) 4 "multi-facet variants"
  assert-equal $manifest.files.".config/demo/multi.txt" {root: $multi_root.index} "multi-facet file"
  for entry in ($multi_root.item.variants | zip [
    "density=compact;tone=dark"
    "density=compact;tone=light"
    "density=roomy;tone=dark"
    "density=roomy;tone=light"
  ]) {
    assert-equal (open --raw ($entry.0 | path join ".config/demo/multi.txt")) $entry.1 "multi-facet contents"
  }

  let static_text = $manifest.files.".config/demo/static.txt"
  assert-equal (open --raw $static_text) "static" "static text contents"

  let static_source = $manifest.files.".config/demo/static-source.nu"
  assert-true ($static_source != $source_path) "static source was not materialized"
  assert-true ($static_source | path exists) "materialized static source is missing"

  for entry in ([dark light] | enumerate) {
    let variant = $entry.item
    let root = $theme_root.item.variants | get $entry.index
    assert-true ($root | path exists) $"($variant) variant root is missing"
    assert-equal (open --raw ($root | path join ".config/demo/dynamic.txt")) $"tone=($variant)" $"($variant) dynamic text"

    let source = $root | path join ".config/demo/dynamic-source.nu"
    let target = ^readlink $source | str trim
    assert-true ($target != $source_path) $"($variant) dynamic source was not materialized"
    assert-true ($target | path exists) $"($variant) dynamic source target is missing"
  }

  assert-equal ($manifest.effects | columns | sort) [dynamic "file:.config/demo/dynamic.txt" "file:.config/demo/static.txt" static] "effects"
  assert-equal ($manifest.effects | get "file:.config/demo/static.txt").on [] "static file effect filter"
  assert-equal ($manifest.effects | get "file:.config/demo/static.txt").exec [/bin/true] "static file effect command"
  assert-equal ($manifest.effects | get "file:.config/demo/dynamic.txt").on [theme] "dynamic file effect filter"
  assert-equal ($manifest.effects | get "file:.config/demo/dynamic.txt").ignoreFailure true "dynamic file effect ignored failure"
  assert-equal $manifest.effects.static.on [] "static effect filter"
  assert-equal $manifest.effects.static.ignoreFailure true "ignored static effect failure"
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
