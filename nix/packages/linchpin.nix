{
  pkgs,
  lib,
  rustPlatform,
  makeBinaryWrapper,
  nix,
  nix-prefetch-git,
  git,
}:

let
  cargoToml = builtins.fromTOML (builtins.readFile ../../rust/linchpin/Cargo.toml);
in
rustPlatform.buildRustPackage (finalAttrs: {
  pname = cargoToml.package.name;
  inherit (cargoToml.package) version;

  src = lib.sourceFilesBySuffices ../../rust/linchpin [
    ".rs"
    ".toml"
    ".lock"
    ".nix"
    ".json" # Test fixtures
  ];

  cargoLock = {
    lockFile = ../../rust/linchpin/Cargo.lock;
  };

  nativeBuildInputs = [
    makeBinaryWrapper
    pkgs.pkg-config
  ];
  buildInputs = [
    pkgs.openssl
    pkgs.sqlite
  ];

  postInstall = ''
    wrapProgram $out/bin/linchpin --prefix PATH : ${
      lib.makeBinPath [
        nix
        nix-prefetch-git
        git
      ]
    }
  '';

  stripAllList = [ "bin" ];

  meta = with lib; {
    homepage = "https://github.com/noi0103/linchpin";
    license = licenses.mit;
    maintainers = with lib.maintainers; [ noi0103 ];
    mainProgram = "linchpin";
  };
})
