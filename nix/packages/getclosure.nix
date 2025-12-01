{ pkgs, ... }: pkgs.writeShellScriptBin "getclosure" (builtins.readFile ../../getclosure.sh)
