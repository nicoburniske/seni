def assert-equal [actual expected message: string] {
  if $actual != $expected {
    error make {msg: $"($message): expected ($expected), got ($actual)"}
  }
}

def main [manifest_path: string] {
  let manifest = open $manifest_path

  assert-equal $manifest.version 4 "manifest version"
  assert-equal $manifest.home "/home/tester" "manifest home"
  assert-equal ($manifest.facets | columns) [theme] "manifest facets"
  assert-equal $manifest.facets.theme.default "light" "facet default"
  assert-equal ($manifest.facets.theme.variants | columns | sort) [dark light] "facet variants"
  for root in ($manifest.facets.theme.variants | values) {
    if not ($root | path exists) {
      error make {msg: $"variant root is missing: ($root)"}
    }
  }

  assert-equal ($manifest.files | columns) [".config/demo/app.conf"] "managed files"
  assert-equal $manifest.files.".config/demo/app.conf" {facet: theme} "managed file source"

  assert-equal ($manifest.effects | columns | sort) [demo generated] "effects"
  assert-equal $manifest.effects.demo.on [] "static effect filter"
  assert-equal $manifest.effects.demo.exec ["/bin/echo" reload] "static effect command"
  assert-equal $manifest.effects.generated.on [theme] "generated effect filter"
  assert-equal $manifest.effects.generated.exec.facet "theme" "generated effect facet"
  assert-equal ($manifest.effects.generated.exec.variants | columns | sort) [dark light] "generated effect variants"
  assert-equal $manifest.effects.generated.exec.variants.dark ["/bin/echo" "tone=dark"] "dark effect command"
  assert-equal $manifest.effects.generated.exec.variants.light ["/bin/echo" "tone=light"] "light effect command"
}
