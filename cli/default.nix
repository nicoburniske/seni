{rustPlatform}:
rustPlatform.buildRustPackage {
  pname = "seni";
  version = "0.1.0";

  src = ./.;
  cargoLock.lockFile = ./Cargo.lock;
}
