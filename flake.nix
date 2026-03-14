{
  description = "Incomplete but functional";

  inputs = {
    # For Nix lib
    nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";
  };

  outputs =
    { self, nixpkgs, ... }:
    let
      eachSystem = nixpkgs.lib.genAttrs [
        "aarch64-darwin"
        "aarch64-linux"
        "x86_64-darwin"
        "x86_64-linux"
      ];
    in
    {
      packages = eachSystem (
        system:
        let
          sources = import ./lon.nix;
          pkgs = import sources.nixpkgs { inherit system; };
          packages = import ./nix/packages { inherit pkgs; };
        in
        (pkgs.lib.recurseIntoAttrs (import ./nix/packages { inherit pkgs; }))
      );

      apps = eachSystem (system: {
        default = {
          type = "app";
          program = nixpkgs.lib.getExe self.packages.${system}.default;
        };
      });

      nixosModules.default = import ./nix/module.nix;
    };
}
