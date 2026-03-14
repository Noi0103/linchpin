{
  system ? builtins.currentSystem,
}:
let
  sources = import ./lon.nix;
  pkgs = import sources.nixpkgs { inherit system; };
  lib = pkgs.lib;
in
rec {
  packages = lib.recurseIntoAttrs (import ./nix/packages { inherit pkgs; });

  checks = lib.recurseIntoAttrs {
    pre-commit = import ./nix/pre-commit.nix;

    tests = lib.recurseIntoAttrs (
      import ./nix/tests {
        inherit pkgs;
        extraBaseModules = {
          module-test = {
            imports = [ ./nix/module.nix ];
            environment.systemPackages = [ packages.linchpin ];
          };
        };
      }
    );
  };
}
