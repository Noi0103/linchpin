{ pkgs }:

rec {
  default = linchpin;
  linchpin = pkgs.callPackage ./linchpin.nix { };
  linchpinTests = pkgs.callPackage ./linchpin-tests.nix { inherit linchpin; };
  getclosure = pkgs.callPackage ./getclosure.nix { };
}
