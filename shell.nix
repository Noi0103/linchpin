let
  sources = import ./lon.nix;
  pkgs = import sources.nixpkgs { };
in
pkgs.mkShell {
  packages = with pkgs; [
    nixfmt-rfc-style
    shellcheck

    # rust
    rustc
    cargo
    rust-analyzer
    rustfmt
    clippy

    # If the dependencies need system libs, you usually need pkg-config + the lib
    pkg-config
    openssl

    # other
    jq
    sqlite
  ];

  inputsFrom = [ (import ./default.nix { }).packages.linchpin ];

  shellHook = ''
    ${(import ./nix/pre-commit.nix).shellHook}
  '';

  RUST_SRC_PATH = "${pkgs.rust.packages.stable.rustPlatform.rustLibSrc}";

  env = {
    RUST_BACKTRACE = "full";

    # gitlab ci shell runner environment
    CI_MERGE_REQUEST_PROJECT_ID = "1229";
    CI_MERGE_REQUEST_IID = "22";
    CI_COMMIT_SHA = "000";
    CI_JOB_NAME = "no_name";
    CI_PIPELINE_ID = "0";
  };
}
