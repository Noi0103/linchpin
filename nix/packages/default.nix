{ pkgs }:

rec {
  default = linchpin;
  linchpin = pkgs.callPackage ./linchpin.nix { };
  linchpinTests = pkgs.callPackage ./linchpin-tests.nix { inherit linchpin; };
  get_closure = pkgs.callPackage ./get_closure.nix { };
}
