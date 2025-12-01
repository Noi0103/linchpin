let
  sources = import ../lon.nix;
  pkgs = import sources.nixpkgs { };
  pre-commit = import sources.pre-commit;
in
pre-commit.run {
  src = ./.;

  hooks = {
    nixfmt-rfc-style.enable = true;
    shellcheck = {
      enable = true;
      args = [
        "-e"
        "SC2148"
        "-e"
        "SC2086"
        "-e"
        "SC2016"
      ];
    };
    clippy = {
      enable = true;
      packageOverrides = {
        cargo = pkgs.cargo;
        clippy = pkgs.clippy;
      };
    };
    rustfmt = {
      enable = true;
      packageOverrides = {
        cargo = pkgs.cargo;
        rustfmt = pkgs.rustfmt;
      };
    };
  };

  settings = {
    rust.cargoManifestPath = "rust/linchpin/Cargo.toml";
  };
}
