{
  pkgs,
}:
let
  self = pkgs.rustPlatform.buildRustPackage {
    pname = "linchpin";
    meta.mainProgram = "linchpin";
    version = "1.0";
    src = pkgs.lib.sourceFilesBySuffices ./. [
      ".rs"
      ".toml"
      ".lock"
    ];
    cargoLock = {
      lockFile = ./Cargo.lock;
    };
    nativeBuildInputs = [ pkgs.pkg-config ];
    buildInputs = [
      pkgs.openssl
      pkgs.sqlite
    ];
  };
in
self
